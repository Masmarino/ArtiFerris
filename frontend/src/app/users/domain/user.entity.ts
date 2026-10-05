import type { InvitationMail } from '../../shared/invitation-mail'

export interface UserSummary {
  id: string
  username: string
  is_super_admin: boolean
  organization_id: string
  email: string | null
  invitation_pending: boolean
  created_at: string
  /** When the pending invitation's link stops working, already past for an expired one; `null` without one. */
  invitation_expires_at: string | null
  /** A confirmed authenticator app or a passkey. */
  mfa_enabled: boolean
}

/** The account just invited, and what became of its activation mail. */
export type InvitedUser = UserSummary & InvitationMail

/** An invited account has no username until it is activated; its address stands in. */
export function displayName(user: Pick<UserSummary, 'username' | 'email' | 'invitation_pending'>) {
  return user.invitation_pending ? (user.email ?? user.username) : user.username
}
