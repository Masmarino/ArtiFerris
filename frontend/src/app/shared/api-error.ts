import { t } from './i18n/translator'
import { HttpErrorResponse } from '@angular/common/http'

/** Same as `badRequestMessage` for a request made with `responseType: 'blob'`, where the error body arrives as a Blob. */
export async function badRequestBlobMessage(error: unknown): Promise<string | null> {
  if (
    !(error instanceof HttpErrorResponse) ||
    error.status !== 400 ||
    !(error.error instanceof Blob)
  ) {
    return null
  }
  try {
    return messageOf(JSON.parse(await error.error.text()))
  } catch {
    return null
  }
}

export function errorCode(error: unknown): string | null {
  return error instanceof HttpErrorResponse ? codeOf(error.error) : null
}

function codeOf(body: unknown): string | null {
  const code = (body as { code?: unknown } | null)?.code
  return typeof code === 'string' && code !== '' ? code : null
}

/**
 * What to tell the user about an error body: the translation of its `code` when there is one, so
 * the wording is ours and in the user's language; otherwise the server's own text (an error that
 * has no code yet).
 */
function messageOf(body: unknown): string | null {
  const code = codeOf(body)
  if (code) {
    const key = `errors.api.${code}`
    const translated = t(key)
    if (translated !== key) {
      return translated
    }
  }
  const message = (body as { error?: unknown } | null)?.error
  return typeof message === 'string' && message.trim() !== '' ? message : null
}

function bodyMessage(error: HttpErrorResponse): string | null {
  return messageOf(error.error)
}

export function badRequestMessage(error: unknown): string | null {
  return error instanceof HttpErrorResponse && error.status === 400 ? bodyMessage(error) : null
}

export interface OverloadMessages {
  tooManyRequests?: string
  busy?: string
  timeout?: string
}

export function overloadMessage(error: unknown, messages: OverloadMessages = {}): string | null {
  if (!(error instanceof HttpErrorResponse)) {
    return null
  }
  switch (error.status) {
    case 429:
      return t(messages.tooManyRequests ?? 'errors.overload.tooManyRequests')
    case 503:
      return t(messages.busy ?? 'errors.overload.busy')
    case 408:
      return t(messages.timeout ?? 'errors.overload.timeout')
    default:
      return null
  }
}

export const secretUnreadableMessage = (): string => t('errors.secretUnreadable')

export function isSecretUnreadable(error: unknown): boolean {
  return error instanceof HttpErrorResponse && error.status === 409
}

export function secretFormFailureMessage(error: unknown, fallback: string): string {
  if (isSecretUnreadable(error)) {
    return secretUnreadableMessage()
  }
  return badRequestMessage(error) ?? overloadMessage(error) ?? fallback
}

/** Like `badRequestMessage`, plus a 409 (a name already taken is worth telling the user) and the overload statuses. */
export function rejectionMessage(error: unknown): string | null {
  return error instanceof HttpErrorResponse && (error.status === 400 || error.status === 409)
    ? bodyMessage(error)
    : overloadMessage(error)
}
