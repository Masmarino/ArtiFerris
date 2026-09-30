import { HttpErrorResponse } from '@angular/common/http'
import {
  badRequestBlobMessage,
  badRequestMessage,
  errorCode,
  isSecretUnreadable,
  overloadMessage,
  rejectionMessage,
} from './api-error'

describe('badRequestMessage', () => {
  it('returns the message the server gave for a 400', () => {
    const error = new HttpErrorResponse({ status: 400, error: { error: 're-enter the secret' } })

    expect(badRequestMessage(error)).toBe('re-enter the secret')
  })

  it.each([
    ['another status', new HttpErrorResponse({ status: 500, error: { error: 'internal error' } })],
    ['a 400 without a body', new HttpErrorResponse({ status: 400 })],
    ['a 400 with an empty message', new HttpErrorResponse({ status: 400, error: { error: ' ' } })],
    [
      'a 400 with a non-string message',
      new HttpErrorResponse({ status: 400, error: { error: 1 } }),
    ],
    ['something that is not an HTTP error', new Error('boom')],
  ])('returns null for %s', (_label, error) => {
    expect(badRequestMessage(error)).toBeNull()
  })
})

describe('rejectionMessage', () => {
  it.each([400, 409])('returns the server message for a %i', (status) => {
    expect(rejectionMessage(new HttpErrorResponse({ status, error: { error: 'taken' } }))).toBe(
      'taken',
    )
  })

  it.each([
    [429, 'Trop de demandes, réessayez dans un instant'],
    [503, 'Service momentanément occupé'],
    [408, 'La requête a pris trop de temps, réessayez'],
  ])('words a %i in French instead of returning the English server text', (status, message) => {
    expect(rejectionMessage(new HttpErrorResponse({ status, error: { error: 'busy' } }))).toBe(
      message,
    )
  })

  it.each([
    ['a 500', new HttpErrorResponse({ status: 500, error: { error: 'internal error' } })],
    [
      'a 409 with a non-string message',
      new HttpErrorResponse({ status: 409, error: { error: {} } }),
    ],
    ['something that is not an HTTP error', { error: { error: 'plain object' } }],
  ])('returns null for %s', (_label, error) => {
    expect(rejectionMessage(error)).toBeNull()
  })
})

describe('overloadMessage', () => {
  const http = (status: number) => new HttpErrorResponse({ status, error: { error: 'busy' } })

  it.each([
    [429, 'Trop de demandes, réessayez dans un instant'],
    [503, 'Service momentanément occupé'],
    [408, 'La requête a pris trop de temps, réessayez'],
  ])('words a %i in French, never with the server text', (status, message) => {
    expect(overloadMessage(http(status))).toBe(message)
  })

  it('lets a caller word each status its own way', () => {
    const messages = { tooManyRequests: 'a', busy: 'b', timeout: 'c' }

    expect([429, 503, 408].map((status) => overloadMessage(http(status), messages))).toEqual([
      'a',
      'b',
      'c',
    ])
  })

  it.each([400, 401, 404, 409, 500])('returns null for a %i', (status) => {
    expect(overloadMessage(http(status))).toBeNull()
  })

  it('returns null for something that is not an HTTP error', () => {
    expect(overloadMessage(new Error('boom'))).toBeNull()
  })
})

describe('isSecretUnreadable', () => {
  it('is true for a 409 only', () => {
    expect(isSecretUnreadable(new HttpErrorResponse({ status: 409 }))).toBe(true)
    expect(isSecretUnreadable(new HttpErrorResponse({ status: 400 }))).toBe(false)
    expect(isSecretUnreadable(new Error('409'))).toBe(false)
  })
})

describe('badRequestBlobMessage', () => {
  const blob = (text: string) => new Blob([text], { type: 'application/json' })

  it('reads the message out of a Blob body of a 400', async () => {
    const error = new HttpErrorResponse({ status: 400, error: blob('{"error":"not exportable"}') })

    expect(await badRequestBlobMessage(error)).toBe('not exportable')
  })

  it.each([
    ['another status', new HttpErrorResponse({ status: 500, error: blob('{"error":"x"}') })],
    ['a body that is not a Blob', new HttpErrorResponse({ status: 400, error: { error: 'x' } })],
    ['a Blob that is not JSON', new HttpErrorResponse({ status: 400, error: blob('<html>') })],
    ['a JSON without a message', new HttpErrorResponse({ status: 400, error: blob('{}') })],
  ])('returns null for %s', async (_label, error) => {
    expect(await badRequestBlobMessage(error)).toBeNull()
  })
})

describe('error codes', () => {
  const failure = (status: number, body: unknown) => new HttpErrorResponse({ status, error: body })

  it('reads the code of the body, and none from anything else', () => {
    expect(errorCode(failure(400, { error: 'x', code: 'username_taken' }))).toBe('username_taken')
    expect(errorCode(failure(400, { error: 'x' }))).toBeNull()
    expect(errorCode(failure(400, { error: 'x', code: 3 }))).toBeNull()
    expect(errorCode(failure(400, { error: 'x', code: '' }))).toBeNull()
    expect(errorCode(failure(400, null))).toBeNull()
    expect(errorCode(new Error('boom'))).toBeNull()
  })

  it('shows our translation of a known code instead of the server wording', () => {
    const error = failure(400, { error: 'username already in use', code: 'username_taken' })

    expect(badRequestMessage(error)).toBe('Ce nom d’utilisateur est déjà pris.')
    expect(rejectionMessage(failure(409, { error: 'x', code: 'repository_name_taken' }))).toBe(
      'Ce nom de dépôt est déjà pris.',
    )
  })

  it('is not affected by a rewording of the server text', () => {
    const reworded = failure(400, {
      error: 'a completely different sentence',
      code: 'invalid_email',
    })

    expect(badRequestMessage(reworded)).toBe('Adresse e-mail invalide.')
  })

  it('falls back to the server text for a code it has no translation for', () => {
    const error = failure(400, { error: 'something new', code: 'a_code_from_the_future' })

    expect(badRequestMessage(error)).toBe('something new')
  })

  it('falls back to the server text when there is no code', () => {
    expect(badRequestMessage(failure(400, { error: 'old server wording' }))).toBe(
      'old server wording',
    )
  })

  it('translates the code of an error body that arrives as a Blob', async () => {
    const body = new Blob([JSON.stringify({ error: 'x', code: 'invalid_smtp_settings' })])

    expect(await badRequestBlobMessage(failure(400, body))).toBe('Paramètres SMTP invalides.')
  })
})
