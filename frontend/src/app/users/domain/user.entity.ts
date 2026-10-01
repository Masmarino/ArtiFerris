export interface UserSummary {
  id: string
  username: string
  is_super_admin: boolean
  organization_id: string
  email: string | null
  invitation_pending: boolean
}

/** An invited account has no username until it is activated; its address stands in. */
export function displayName(user: Pick<UserSummary, 'username' | 'email' | 'invitation_pending'>) {
  return user.invitation_pending ? (user.email ?? user.username) : user.username
}
