import { InjectionToken } from '@angular/core'
import { Observable } from 'rxjs'
import { OrganizationMember } from '../domain/organization-member.entity'

export interface OrganizationMembersPort {
  list(organizationId: string): Observable<OrganizationMember[]>
  invite(
    organizationId: string,
    email: string,
    isOrganizationAdmin: boolean,
  ): Observable<OrganizationMember>
  setOrganizationAdmin(
    organizationId: string,
    userId: string,
    isOrganizationAdmin: boolean,
  ): Observable<void>
}

export const ORGANIZATION_MEMBERS_PORT = new InjectionToken<OrganizationMembersPort>(
  'OrganizationMembersPort',
)
