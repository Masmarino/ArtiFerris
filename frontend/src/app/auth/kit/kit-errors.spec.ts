import { HttpErrorResponse } from '@angular/common/http'
import {
  classifyActivateFailure,
  classifyMfaFailure,
  classifyPasswordFailure,
  classifyRegisterFailure,
  isTooManyPasskeys,
} from '@masmarino/gabarit/auth'
import { toKitError } from './kit-errors'

const api = (status: number, error: string, code?: string) =>
  new HttpErrorResponse({ status, error: code ? { error, code } : { error } })

describe('toKitError', () => {
  it.each([
    ['username_taken', 'username-taken'],
    ['email_taken', 'email-taken'],
    ['invalid_username', 'username-invalid'],
    ['reserved_name', 'username-reserved'],
    ['invalid_email', 'email-invalid'],
    ['password_too_short', 'password-weak'],
    ['registration_disabled', 'disabled'],
    ['registration_unavailable', 'disabled'],
  ])('lets the registration read our %s as %s', (code, failure) => {
    expect(classifyRegisterFailure(toKitError(api(400, 'any wording', code)))).toBe(failure)
  })

  it.each([
    ['username_taken', 'username-taken'],
    ['invalid_username', 'username-invalid'],
    ['reserved_name', 'username-reserved'],
    ['password_too_short', 'weak-password'],
    ['invitation_not_found', 'invalid-link'],
    ['invitation_expired', 'invalid-link'],
  ])('lets the activation read our %s as %s', (code, failure) => {
    expect(classifyActivateFailure(toKitError(api(400, 'any wording', code)))).toBe(failure)
  })

  it.each([
    [api(400, 'any wording', 'invalid_mfa_code'), 'wrong-code'],
    [api(400, 'any wording', 'mfa_already_enabled'), 'already-set-up'],
    [api(400, 'any wording', 'mfa_enrollment_expired'), 'expired'],
    [api(401, 'invalid or expired mfa token'), 'expired'],
    [api(401, 'invalid code'), 'wrong-code'],
    [api(429, 'too many failed attempts, try again later'), 'rate-limited'],
    [api(503, 'passkeys are not available', 'passkeys_unavailable'), 'unavailable'],
  ])('lets the second factor read %o as %s', (error, failure) => {
    expect(classifyMfaFailure(toKitError(error))).toBe(failure)
  })

  it('lets the security settings read a wrong current password', () => {
    expect(
      classifyPasswordFailure(toKitError(api(400, 'invalid credentials', 'invalid_credentials'))),
    ).toBe('wrong-password')
  })

  it('lets the security settings read an account at its passkey limit', () => {
    expect(
      isTooManyPasskeys(
        toKitError(api(400, 'an account can hold at most 10 passkeys', 'validation')),
      ),
    ).toBe(true)
  })

  it('keeps the status and the body of the errors the kit does not react to', () => {
    expect(toKitError(api(500, 'internal error', 'storage_failure'))).toEqual({
      status: 500,
      error: { error: 'internal error', code: 'storage_failure' },
    })
  })

  it('leaves what is not an HTTP error alone', () => {
    const error = new Error('offline')

    expect(toKitError(error)).toBe(error)
  })
})
