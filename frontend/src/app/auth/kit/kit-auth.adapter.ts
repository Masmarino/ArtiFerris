import { Injectable, inject } from '@angular/core'
import type {
  AuthConfig,
  AuthPort as KitAuthPort,
  LoginResponse as KitLoginResponse,
  MfaProof,
  MfaSetupResult,
  PasskeyChallenge,
  TotpEnrollment,
} from '@masmarino/gabarit/auth'
import { Observable, defer, map, share, tap } from 'rxjs'
import { AUTH_PORT } from '../application/auth.port'
import { SessionToken } from '../application/session-token'
import { LoginResponse, SsoConfig } from '../domain/auth.types'
import { kitErrors } from './kit-errors'

const toKitLogin = (response: LoginResponse): KitLoginResponse => ({
  token: response.token,
  mfaToken: response.mfa_token ?? undefined,
  mfaSetupRequired: response.mfa_setup_required,
  mfaHasTotp: response.mfa_has_totp,
  mfaHasPasskey: response.mfa_has_passkey,
})

const toChallenge = (start: { challenge_id: string; public_key: unknown }): PasskeyChallenge => ({
  challengeId: start.challenge_id,
  publicKey: start.public_key,
})

/**
 * Gabarit's auth kit on our API: camelCase over our snake_case, LDAP sign-in when the organisation
 * uses it, our error codes as the kit reads them. Like the kit expects, a session is stored when a
 * sign-in completes (no second factor, a verified challenge, a registration without one), never by
 * the first enrolment: the kit calls `setToken` once the backup codes are acknowledged.
 */
@Injectable({ providedIn: 'root' })
export class KitAuthAdapter implements KitAuthPort {
  private readonly port = inject(AUTH_PORT)
  private readonly session = inject(SessionToken)
  private ssoType: SsoConfig['type'] = null

  /**
   * The organisation's sign-in settings, one request for the callers that ask at the same time (the
   * sign-in page and the kit's form). A later call asks again.
   */
  readonly ssoConfig$: Observable<SsoConfig> = defer(() => this.port.getSsoConfig()).pipe(
    tap((config) => (this.ssoType = config.type)),
    share(),
  )

  authConfig(): Observable<AuthConfig> {
    return this.ssoConfig$.pipe(
      map((config) => ({
        registrationEnabled: config.registration_enabled,
        passkeysAvailable: true,
      })),
    )
  }

  login(username: string, password: string): Observable<KitLoginResponse> {
    const attempt =
      this.ssoType === 'ldap'
        ? this.port.loginWithLdap(username, password)
        : this.port.login(username, password)
    return attempt.pipe(
      kitErrors,
      map(toKitLogin),
      tap((response) => this.store(response.token)),
    )
  }

  register(username: string, email: string, password: string): Observable<KitLoginResponse> {
    return this.port.register(username, email, password).pipe(
      kitErrors,
      map(toKitLogin),
      tap((response) => this.store(response.token)),
    )
  }

  /** The invitee always chooses their username here: the administrator invites by e-mail only. */
  activate(token: string, password: string, username = ''): Observable<void> {
    return this.port.activate(token, username, password).pipe(kitErrors)
  }

  resetPassword(token: string, password: string): Observable<void> {
    return this.port.resetPassword(token, password).pipe(kitErrors)
  }

  verifyMfa(mfaToken: string, proof: MfaProof): Observable<void> {
    const [code, backupCode] = 'code' in proof ? [proof.code] : [undefined, proof.backupCode]
    return this.port.verifyMfa(mfaToken, code, backupCode).pipe(
      kitErrors,
      tap((response) => this.store(response.token)),
      map(() => undefined),
    )
  }

  startPasskeyChallenge(mfaToken: string): Observable<PasskeyChallenge> {
    return this.port.startMfaPasskey(mfaToken).pipe(kitErrors, map(toChallenge))
  }

  finishPasskeyChallenge(
    mfaToken: string,
    challengeId: string,
    credential: unknown,
  ): Observable<void> {
    return this.port.finishMfaPasskey(mfaToken, challengeId, credential).pipe(
      kitErrors,
      tap((response) => this.store(response.token)),
      map(() => undefined),
    )
  }

  enrollTotp(mfaToken: string): Observable<TotpEnrollment> {
    return this.port.startTotpSetup(mfaToken).pipe(
      kitErrors,
      map((enrollment) => ({ secret: enrollment.secret, otpauthUrl: enrollment.otpauth_url })),
    )
  }

  confirmTotp(mfaToken: string, code: string): Observable<MfaSetupResult> {
    return this.port.confirmTotpSetup(mfaToken, code).pipe(
      kitErrors,
      map((result) => ({ token: result.token, backupCodes: result.backup_codes })),
    )
  }

  startPasskeySetup(mfaToken: string): Observable<PasskeyChallenge> {
    return this.port.startPasskeySetup(mfaToken).pipe(kitErrors, map(toChallenge))
  }

  finishPasskeySetup(
    mfaToken: string,
    challengeId: string,
    credential: unknown,
    name: string,
  ): Observable<MfaSetupResult> {
    return this.port.finishPasskeySetup(mfaToken, challengeId, credential, name).pipe(
      kitErrors,
      map((result) => ({ token: result.token, backupCodes: result.backup_codes })),
    )
  }

  setToken(token: string): void {
    this.session.set(token)
  }

  private store(token: string | null): void {
    if (token) {
      this.session.set(token)
    }
  }
}
