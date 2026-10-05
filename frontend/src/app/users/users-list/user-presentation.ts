import type { BadgeVariant } from '@masmarino/gabarit/badge'
import { t } from '../../shared/i18n/translator'
import { errorCode } from '../../shared/api-error'
import { RowDate, exactTime, rowDate } from '../../shared/row-date'
import { UserSummary } from '../domain/user.entity'

/** A badge: what it says, its tone, its icon. The same as FerrisGit's administration shows. */
export interface Presentation {
  label: string
  variant: BadgeVariant
  icon: string
}

const presentation =
  (labelKey: string, variant: BadgeVariant, icon: string) => (): Presentation => ({
    label: t(labelKey),
    variant,
    icon,
  })

const ACTIVE = presentation('users.state.active', 'success', 'circle-check')
const PENDING = presentation('users.state.pending', 'warning', 'clock')
const EXPIRED = presentation('users.state.expired', 'error', 'alert-circle')
const MFA_ON = presentation('users.mfa.on', 'success', 'shield-check')
const MFA_OFF = presentation('users.mfa.off', 'neutral', 'shield-alert')
export const SUPER_ADMIN = presentation('users.superAdmin', 'info', 'shield-check')

export function accountState(
  user: UserSummary,
  now: Date,
): { state: Presentation; expiry: RowDate | null } {
  if (!user.invitation_pending) {
    return { state: ACTIVE(), expiry: null }
  }
  const expiresAt = user.invitation_expires_at
  if (!expiresAt) {
    return { state: PENDING(), expiry: null }
  }
  if (new Date(expiresAt).getTime() <= now.getTime()) {
    return {
      state: EXPIRED(),
      expiry: rowDate(expiresAt, now, 'users.row.expiredAgo', 'users.row.expiredOn'),
    }
  }
  return {
    state: PENDING(),
    expiry: {
      iso: expiresAt,
      title: exactTime(expiresAt),
      label: t('users.row.expires', { date: exactTime(expiresAt) }),
    },
  }
}

/** An invited account has no second factor to speak of yet. */
export const mfaPresentation = (user: UserSummary): Presentation | null =>
  user.invitation_pending ? null : user.mfa_enabled ? MFA_ON() : MFA_OFF()

export const adminAction = (isSuperAdmin: boolean) =>
  isSuperAdmin
    ? { label: t('users.actions.demote'), icon: 'shield-off' }
    : { label: t('users.actions.promote'), icon: 'shield-plus' }

/** What resetting the second factors does, said to the administrator before they confirm. */
export const resetMfaMessage = (name: string, isSelf: boolean) =>
  isSelf ? t('users.mfaReset.selfMessage') : t('users.mfaReset.message', { username: name })

export const resetMfaFailure = (error: unknown) =>
  errorCode(error) === 'mfa_not_enrolled'
    ? t('users.mfaReset.errors.notEnrolled')
    : t('users.mfaReset.errors.failed')

export const resetPasswordFailure = (error: unknown) => {
  switch (errorCode(error)) {
    case 'own_password_reset':
      return t('users.passwordReset.errors.own')
    case 'account_not_activated':
      return t('users.passwordReset.errors.notActivated')
    case 'password_managed_by_identity_provider':
      return t('users.passwordReset.errors.identityProvider')
    default:
      return t('users.passwordReset.errors.failed')
  }
}
