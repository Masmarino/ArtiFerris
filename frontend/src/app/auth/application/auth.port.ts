import { InjectionToken } from '@angular/core'
import { Observable } from 'rxjs'
import {
  LoginResponse,
  SsoConfig,
  TotpSetupComplete,
  TotpSetupEnrollment,
} from '../domain/auth.types'

export interface AuthPort {
  login(username: string, password: string): Observable<LoginResponse>
  register(username: string, email: string, password: string): Observable<LoginResponse>
  getSsoConfig(): Observable<SsoConfig>
  loginWithLdap(username: string, password: string): Observable<LoginResponse>
  activate(token: string, username: string, newPassword: string): Observable<void>
  /** Sets a new password through the link an administrator issued. No session: the person signs in afterwards. */
  resetPassword(token: string, newPassword: string): Observable<void>
  /** Revokes every session, Docker token and API token of the caller, this one included. */
  logoutAll(): Observable<void>
  verifyMfa(mfaToken: string, code?: string, backupCode?: string): Observable<LoginResponse>
  // WebAuthn options from the server, passed to the browser as is.
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  startMfaPasskey(mfaToken: string): Observable<{ challenge_id: string; public_key: any }>
  finishMfaPasskey(
    mfaToken: string,
    challengeId: string,
    credential: unknown,
  ): Observable<LoginResponse>
  startTotpSetup(mfaToken: string): Observable<TotpSetupEnrollment>
  confirmTotpSetup(mfaToken: string, code: string): Observable<TotpSetupComplete>
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  startPasskeySetup(mfaToken: string): Observable<{ challenge_id: string; public_key: any }>
  /** The first passkey comes with the session and the backup codes, like a confirmed app. */
  finishPasskeySetup(
    mfaToken: string,
    challengeId: string,
    credential: unknown,
    name: string,
  ): Observable<TotpSetupComplete>
}

export const AUTH_PORT = new InjectionToken<AuthPort>('AuthPort')
