import type { InvitationMail } from '../../shared/invitation-mail'

export interface OrganizationMember {
  id: string
  username: string
  email: string | null
  is_organization_admin: boolean
  invitation_pending: boolean
}

/** The member just invited, and what became of their activation mail. */
export type InvitedMember = OrganizationMember & InvitationMail
