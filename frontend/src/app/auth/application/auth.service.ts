import { Injectable, computed, inject } from '@angular/core'
import { Observable } from 'rxjs'
import { AUTH_PORT } from './auth.port'
import { SessionToken } from './session-token'
import { consumeSsoStart, discardSsoStart, markSsoStarted } from './sso-handshake'
import { safeReturnUrl } from '../domain/return-url'
import { PageTitleService } from '../../shell/page-title.service'
import { ToastService } from '../../shared/toast.service'

@Injectable({ providedIn: 'root' })
export class AuthService {
  private readonly port = inject(AUTH_PORT)
  private readonly session = inject(SessionToken)
  private readonly pageTitle = inject(PageTitleService)
  private readonly toasts = inject(ToastService)

  readonly token = this.session.value
  readonly isAuthenticated = computed(() => this.token() !== null)

  logoutEverywhere(): Observable<void> {
    return this.port.logoutAll()
  }

  logout(): void {
    this.session.clear()
    // A private page name or toast must not outlive the session.
    this.pageTitle.title.set('')
    this.toasts.clear()
  }

  beginSsoLogin(returnUrl: string | null): void {
    markSsoStarted(returnUrl)
  }

  abandonSsoLogin(): void {
    discardSsoStart()
  }

  /**
   * OIDC return. Refused unless this tab started the login, so a pasted `#token=` link signs nobody
   * in.
   */
  completeExternalLogin(token: string): { returnUrl: string | null } | null {
    const start = consumeSsoStart()
    if (!start) {
      return null
    }
    this.session.set(token)
    return { returnUrl: safeReturnUrl(start.returnUrl) }
  }
}
