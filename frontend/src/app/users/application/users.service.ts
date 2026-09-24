import { Injectable, inject } from '@angular/core'
import { Observable, tap } from 'rxjs'
import { UserSummary } from '../domain/user.entity'
import { USER_PORT } from './user.port'
import { TokenScopedCache } from '../../shared/token-scoped-cache'

@Injectable({ providedIn: 'root' })
export class UsersService {
  private readonly port = inject(USER_PORT)

  private readonly listCache = new TokenScopedCache<UserSummary[]>(() => this.port.list())

  // cached across callers, cleared by any mutation below — forceRefresh is for the shell's
  // search, which needs to see writes that could've come from another tab
  list(options?: { forceRefresh?: boolean }): Observable<UserSummary[]> {
    return this.listCache.get(options?.forceRefresh)
  }

  get(id: string): Observable<UserSummary> {
    return this.port.get(id)
  }

  create(username: string, email: string, isSuperAdmin: boolean): Observable<UserSummary> {
    return this.port.create(username, email, isSuperAdmin).pipe(tap(() => this.listCache.clear()))
  }

  delete(id: string): Observable<void> {
    return this.port.delete(id).pipe(tap(() => this.listCache.clear()))
  }

  setSuperAdmin(id: string, isSuperAdmin: boolean): Observable<void> {
    return this.port.setSuperAdmin(id, isSuperAdmin).pipe(tap(() => this.listCache.clear()))
  }

  resendInvitation(id: string): Observable<void> {
    return this.port.resendInvitation(id)
  }
}
