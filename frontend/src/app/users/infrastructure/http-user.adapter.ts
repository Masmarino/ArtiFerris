import { Injectable, inject } from '@angular/core'
import { HttpClient } from '@angular/common/http'
import { Observable } from 'rxjs'
import { InvitedUser, UserSummary } from '../domain/user.entity'
import { InvitationMail, PasswordResetMail } from '../../shared/invitation-mail'
import { UserPort } from '../application/user.port'
import { apiPath } from '../../shared/api-path'

@Injectable()
export class HttpUserAdapter implements UserPort {
  private readonly http = inject(HttpClient)

  list(): Observable<UserSummary[]> {
    return this.http.get<UserSummary[]>('/api/users')
  }

  get(id: string): Observable<UserSummary> {
    return this.http.get<UserSummary>(apiPath`/api/users/${id}`)
  }

  create(email: string, isSuperAdmin: boolean): Observable<InvitedUser> {
    return this.http.post<InvitedUser>('/api/users', {
      email,
      is_super_admin: isSuperAdmin,
    })
  }

  delete(id: string): Observable<void> {
    return this.http.delete<void>(apiPath`/api/users/${id}`)
  }

  setSuperAdmin(id: string, isSuperAdmin: boolean): Observable<void> {
    return this.http.put<void>(apiPath`/api/users/${id}/super-admin`, {
      is_super_admin: isSuperAdmin,
    })
  }

  resetPassword(id: string): Observable<PasswordResetMail> {
    return this.http.post<PasswordResetMail>(apiPath`/api/users/${id}/reset-password`, {})
  }

  resetMfa(id: string): Observable<void> {
    return this.http.delete<void>(apiPath`/api/users/${id}/mfa`)
  }

  resendInvitation(id: string): Observable<InvitationMail> {
    return this.http.post<InvitationMail>(apiPath`/api/users/${id}/resend-invitation`, {})
  }
}
