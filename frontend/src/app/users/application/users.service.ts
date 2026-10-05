import { Injectable, inject } from '@angular/core'
import { Observable, tap } from 'rxjs'
import { InvitedUser, UserSummary } from '../domain/user.entity'
import { InvitationMail, PasswordResetMail } from '../../shared/invitation-mail'
import { USER_PORT } from './user.port'
import { TokenScopedCache } from '../../shared/token-scoped-cache'

@Injectable({ providedIn: 'root' })
export class UsersService {
  private readonly port = inject(USER_PORT)

  private readonly listCache = new TokenScopedCache<UserSummary[]>(() => this.port.list())

  // Cached across callers and cleared by any mutation; forceRefresh is for the shell search, which
  // must see writes from another tab.
  list(options?: { forceRefresh?: boolean }): Observable<UserSummary[]> {
    return this.listCache.get(options?.forceRefresh)
  }

  get(id: string): Observable<UserSummary> {
    return this.port.get(id)
  }

  create(email: string, isSuperAdmin: boolean): Observable<InvitedUser> {
    return this.port.create(email, isSuperAdmin).pipe(tap(() => this.listCache.clear()))
  }

  delete(id: string): Observable<void> {
    return this.port.delete(id).pipe(tap(() => this.listCache.clear()))
  }

  setSuperAdmin(id: string, isSuperAdmin: boolean): Observable<void> {
    return this.port.setSuperAdmin(id, isSuperAdmin).pipe(tap(() => this.listCache.clear()))
  }

  resetPassword(id: string): Observable<PasswordResetMail> {
    return this.port.resetPassword(id)
  }

  // The list shows who has a second factor: it changes.
  resetMfa(id: string): Observable<void> {
    return this.port.resetMfa(id).pipe(tap(() => this.listCache.clear()))
  }

  resendInvitation(id: string): Observable<InvitationMail> {
    return this.port.resendInvitation(id)
  }
}
