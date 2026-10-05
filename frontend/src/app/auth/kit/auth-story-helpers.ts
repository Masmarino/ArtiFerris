import { HttpErrorResponse } from '@angular/common/http'
import type { Provider } from '@angular/core'
import { Observable, NEVER, of, throwError, timer, switchMap } from 'rxjs'
import { AUTH_PORT, AuthPort } from '../application/auth.port'
import { LoginResponse, SsoConfig } from '../domain/auth.types'

/** Our API as the sign-in stories and specs see it: every call answers, unless a story says otherwise. */
export function fakeAuthPort(overrides: Partial<AuthPort> = {}): AuthPort {
  return {
    getSsoConfig: () => of<SsoConfig>({ type: null, registration_enabled: true }),
    login: () => of(session()),
    loginWithLdap: () => of(session()),
    register: () => of(mfaPending({ mfa_setup_required: true })),
    activate: () => of(undefined),
    resetPassword: () => of(undefined),
    logoutAll: () => of(undefined),
    verifyMfa: () => of(session()),
    startMfaPasskey: () => NEVER,
    finishMfaPasskey: () => NEVER,
    startTotpSetup: () =>
      of({
        secret: 'JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP',
        otpauth_url:
          'otpauth://totp/ArtiFerris:florian?secret=JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP&issuer=ArtiFerris',
      }),
    confirmTotpSetup: () => of({ token: 'session-jwt', backup_codes: BACKUP_CODES }),
    startPasskeySetup: () => NEVER,
    finishPasskeySetup: () => NEVER,
    ...overrides,
  }
}

export const withAuthPort = (port: AuthPort): Provider => ({ provide: AUTH_PORT, useValue: port })

export const BACKUP_CODES = Array.from({ length: 10 }, (_, i) =>
  `${i}a1b2c3d4e5f60718`.slice(0, 16),
)

export const session = (): LoginResponse => ({
  token: 'session-jwt',
  mfa_token: null,
  mfa_setup_required: false,
  mfa_has_totp: false,
  mfa_has_passkey: false,
})

export const mfaPending = (fields: Partial<LoginResponse> = {}): LoginResponse => ({
  token: null,
  mfa_token: 'mfa-token',
  mfa_setup_required: false,
  mfa_has_totp: true,
  mfa_has_passkey: false,
  ...fields,
})

/** A refusal of our API: its status, its English text and its stable `code`. */
export const apiError = (status: number, error: string, code?: string) =>
  new HttpErrorResponse({ status, error: code ? { error, code } : { error } })

/** Answers a little later, so a story shows the request in flight first. */
export const later = <T>(answer: () => Observable<T>, ms = 300): Observable<T> =>
  timer(ms).pipe(switchMap(answer))

export const refused = (status: number, error: string, code?: string) =>
  later(() => throwError(() => apiError(status, error, code)))
