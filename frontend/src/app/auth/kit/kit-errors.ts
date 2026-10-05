import { HttpErrorResponse } from '@angular/common/http'
import { AUTH_PORT_ERROR_BODIES } from '@masmarino/gabarit/auth'
import { Observable, catchError, throwError } from 'rxjs'

/** What Gabarit's auth kit reads from a failure: a status, and a body whose `error` it matches word for word. */
export interface KitError {
  status: number
  error: unknown
}

const body = (error: string) => ({ error })
const as = (status: number, error: string): KitError => ({ status, error: body(error) })

/**
 * Our API names its errors with a stable `code`; the kit recognises a few by status and English body
 * (`AUTH_PORT_ERROR_BODIES`). The ones it reacts to are rewritten here, so a page points at the right
 * field and words the failure in the user's language; the others keep their status and body.
 */
const BY_CODE: Record<string, KitError> = {
  username_taken: as(409, 'username already in use'),
  email_taken: as(409, AUTH_PORT_ERROR_BODIES.emailTaken),
  invalid_username: as(400, `${AUTH_PORT_ERROR_BODIES.usernamePrefix}is invalid`),
  reserved_name: as(400, AUTH_PORT_ERROR_BODIES.usernameReserved),
  invalid_email: as(400, AUTH_PORT_ERROR_BODIES.invalidEmail),
  password_too_short: as(400, `${AUTH_PORT_ERROR_BODIES.weakPasswordPrefix} 8 characters`),
  registration_disabled: as(400, AUTH_PORT_ERROR_BODIES.registrationDisabled),
  registration_unavailable: as(400, AUTH_PORT_ERROR_BODIES.registrationDisabled),
  invitation_not_found: as(400, AUTH_PORT_ERROR_BODIES.invalidToken),
  invitation_expired: as(400, AUTH_PORT_ERROR_BODIES.invalidToken),
  password_reset_link_invalid: as(400, AUTH_PORT_ERROR_BODIES.invalidToken),
  mfa_already_enabled: as(400, AUTH_PORT_ERROR_BODIES.alreadySetUp),
  invalid_mfa_code: as(400, AUTH_PORT_ERROR_BODIES.invalidCode),
  mfa_enrollment_expired: as(401, AUTH_PORT_ERROR_BODIES.invalidToken),
  // The password asked again in the security settings (a sign-in answers 401 without this code).
  invalid_credentials: as(400, AUTH_PORT_ERROR_BODIES.wrongPassword),
}

/** Our text for an account at its passkey limit (a `validation` error, with the limit in it). */
const PASSKEY_LIMIT = 'an account can hold at most'

/** The 401 of a second-factor step whose mfa token is no longer valid: the kit's "sign in again". */
const EXPIRED_MFA_TOKEN = 'invalid or expired mfa token'

export function toKitError(error: unknown): unknown {
  if (!(error instanceof HttpErrorResponse)) {
    return error
  }
  const payload = error.error as { code?: unknown; error?: unknown } | null
  const code = typeof payload?.code === 'string' ? payload.code : null
  if (code !== null && Object.hasOwn(BY_CODE, code)) {
    return BY_CODE[code]
  }
  if (error.status === 401 && payload?.error === EXPIRED_MFA_TOKEN) {
    return as(401, AUTH_PORT_ERROR_BODIES.invalidToken)
  }
  if (typeof payload?.error === 'string' && payload.error.startsWith(PASSKEY_LIMIT)) {
    return as(400, AUTH_PORT_ERROR_BODIES.tooManyPasskeys)
  }
  return { status: error.status, error: payload } satisfies KitError
}

export const kitErrors = <T>(source: Observable<T>): Observable<T> =>
  source.pipe(catchError((error: unknown) => throwError(() => toKitError(error))))
