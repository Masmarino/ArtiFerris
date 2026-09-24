import { Injectable, computed, inject } from '@angular/core'
import { Observable, map, tap } from 'rxjs'
import {
  LoginOutcome,
  LoginResponse,
  SsoConfig,
  TotpSetupComplete,
  TotpSetupEnrollment,
} from '../domain/auth.types'
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

  login(username: string, password: string): Observable<LoginOutcome> {
    return this.port.login(username, password).pipe(map((response) => this.toOutcome(response)))
  }

  register(username: string, email: string, password: string): Observable<LoginOutcome> {
    return this.port
      .register(username, email, password)
      .pipe(map((response) => this.toOutcome(response)))
  }

  getSsoConfig(): Observable<SsoConfig> {
    return this.port.getSsoConfig()
  }

  loginWithLdap(username: string, password: string): Observable<LoginOutcome> {
    return this.port
      .loginWithLdap(username, password)
      .pipe(map((response) => this.toOutcome(response)))
  }

  private toOutcome(response: LoginResponse): LoginOutcome {
    if (response.token) {
      this.setToken(response.token)
      return { mfaRequired: false }
    }
    return {
      mfaRequired: true,
      mfaToken: response.mfa_token ?? undefined,
      mfaSetupRequired: response.mfa_setup_required,
      mfaHasTotp: response.mfa_has_totp,
      mfaHasPasskey: response.mfa_has_passkey,
    }
  }

  verifyMfa(mfaToken: string, code?: string, backupCode?: string): Observable<void> {
    return this.port.verifyMfa(mfaToken, code, backupCode).pipe(
      tap((response) => this.setToken(response.token!)),
      map(() => undefined),
    )
  }

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  startMfaPasskey(mfaToken: string): Observable<{ challenge_id: string; public_key: any }> {
    return this.port.startMfaPasskey(mfaToken)
  }

  finishMfaPasskey(mfaToken: string, challengeId: string, credential: unknown): Observable<void> {
    return this.port.finishMfaPasskey(mfaToken, challengeId, credential).pipe(
      tap((response) => this.setToken(response.token!)),
      map(() => undefined),
    )
  }

  // --- Mandatory first-time enrollment (account has no second factor yet) ---

  startTotpSetup(mfaToken: string): Observable<TotpSetupEnrollment> {
    return this.port.startTotpSetup(mfaToken)
  }

  confirmTotpSetup(mfaToken: string, code: string): Observable<TotpSetupComplete> {
    return this.port
      .confirmTotpSetup(mfaToken, code)
      .pipe(tap((response) => this.setToken(response.token)))
  }

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  startPasskeySetup(mfaToken: string): Observable<{ challenge_id: string; public_key: any }> {
    return this.port.startPasskeySetup(mfaToken)
  }

  finishPasskeySetup(
    mfaToken: string,
    challengeId: string,
    credential: unknown,
    name: string,
  ): Observable<void> {
    return this.port.finishPasskeySetup(mfaToken, challengeId, credential, name).pipe(
      tap((response) => this.setToken(response.token!)),
      map(() => undefined),
    )
  }

  activate(token: string, newPassword: string): Observable<void> {
    return this.port.activate(token, newPassword)
  }

  /** Ends every session server-side; the caller then drops its own token. */
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

  /** The login page opened with no return trip, so an earlier SSO start was abandoned. */
  abandonSsoLogin(): void {
    discardSsoStart()
  }

  /** OIDC return trip. Refused unless this tab started the SSO login, so a pasted `#token=` link signs nobody in. */
  completeExternalLogin(token: string): { returnUrl: string | null } | null {
    const start = consumeSsoStart()
    if (!start) {
      return null
    }
    this.setToken(token)
    return { returnUrl: safeReturnUrl(start.returnUrl) }
  }

  private setToken(token: string): void {
    this.session.set(token)
  }
}
