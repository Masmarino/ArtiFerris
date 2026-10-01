import { InjectionToken } from '@angular/core'
import { Observable } from 'rxjs'
import { UserSummary } from '../domain/user.entity'

export interface UserPort {
  list(): Observable<UserSummary[]>
  get(id: string): Observable<UserSummary>
  create(email: string, isSuperAdmin: boolean): Observable<UserSummary>
  delete(id: string): Observable<void>
  setSuperAdmin(id: string, isSuperAdmin: boolean): Observable<void>
  resendInvitation(id: string): Observable<void>
}

export const USER_PORT = new InjectionToken<UserPort>('UserPort')
