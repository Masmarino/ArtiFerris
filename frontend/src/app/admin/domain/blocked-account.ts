import { t } from '../../shared/i18n/translator'
import { BlockedAccount } from './audit.entity'

export interface BlockedAccountView {
  /** What the row shows: the username for a login key, the raw key for anything else. */
  label: string
  /** The name to unlock by, or null when the key isn't tied to a username. */
  unlockAs: string | null
}

const SHARED_LOGIN_KEY = 'login-user:'
// `login-org-user:<36-char organization id>:<username>`
const ORGANIZATION_LOGIN_KEY = /^login-org-user:.{36}:(.+)$/s

/** The blocked list returns raw throttle keys; only login keys can be cleared by username. */
export function describeBlockedAccount(account: BlockedAccount): BlockedAccountView {
  const key = account.username
  if (key.startsWith(SHARED_LOGIN_KEY) && key.length > SHARED_LOGIN_KEY.length) {
    const username = key.slice(SHARED_LOGIN_KEY.length)
    return { label: username, unlockAs: username }
  }
  const username = ORGANIZATION_LOGIN_KEY.exec(key)?.[1]
  if (username) {
    return { label: t('admin.securityLog.organizationAccount', { username }), unlockAs: username }
  }
  return { label: key, unlockAs: null }
}
