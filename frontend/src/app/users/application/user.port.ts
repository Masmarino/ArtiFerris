import { InjectionToken } from '@angular/core'
import { Observable } from 'rxjs'
import { InvitedUser, UserSummary } from '../domain/user.entity'
import { InvitationMail, PasswordResetMail } from '../../shared/invitation-mail'

export interface UserPort {
  list(): Observable<UserSummary[]>
  get(id: string): Observable<UserSummary>
  create(email: string, isSuperAdmin: boolean): Observable<InvitedUser>
  delete(id: string): Observable<void>
  setSuperAdmin(id: string, isSuperAdmin: boolean): Observable<void>
  resendInvitation(id: string): Observable<InvitationMail>
  /** Removes every second factor of the account and signs it out everywhere. */
  resetMfa(id: string): Observable<void>
  /** Voids the password and issues a link to choose a new one; the link comes back when its mail could not go out. */
  resetPassword(id: string): Observable<PasswordResetMail>
}

export const USER_PORT = new InjectionToken<UserPort>('UserPort')
