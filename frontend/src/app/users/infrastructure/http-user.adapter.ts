import { Injectable, inject } from '@angular/core'
import { HttpClient } from '@angular/common/http'
import { Observable } from 'rxjs'
import { UserSummary } from '../domain/user.entity'
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

  create(username: string, email: string, isSuperAdmin: boolean): Observable<UserSummary> {
    return this.http.post<UserSummary>('/api/users', {
      username,
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

  resendInvitation(id: string): Observable<void> {
    return this.http.post<void>(apiPath`/api/users/${id}/resend-invitation`, {})
  }
}
