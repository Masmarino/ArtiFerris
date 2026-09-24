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
    const message = (JSON.parse(await error.error.text()) as { error?: unknown } | null)?.error
    return typeof message === 'string' && message.trim() !== '' ? message : null
  } catch {
    return null
  }
}

function bodyMessage(error: HttpErrorResponse): string | null {
  const message = (error.error as { error?: unknown } | null)?.error
  return typeof message === 'string' && message.trim() !== '' ? message : null
}

/** The server's own explanation of a 400, or null for any other failure. */
export function badRequestMessage(error: unknown): string | null {
  return error instanceof HttpErrorResponse && error.status === 400 ? bodyMessage(error) : null
}

export const TOO_MANY_REQUESTS_MESSAGE = 'Trop de demandes, réessayez dans un instant'
export const BUSY_MESSAGE = 'Service momentanément occupé'
export const TIMEOUT_MESSAGE = 'La requête a pris trop de temps, réessayez'
export const SECRET_UNREADABLE_MESSAGE =
  'Le secret enregistré est illisible : saisissez-le à nouveau'

export interface OverloadMessages {
  tooManyRequests?: string
  busy?: string
  timeout?: string
}

/** A 429, 503 or 408: the server is limiting or shedding load, which is not the same as failing. */
export function overloadMessage(error: unknown, messages: OverloadMessages = {}): string | null {
  if (!(error instanceof HttpErrorResponse)) {
    return null
  }
  switch (error.status) {
    case 429:
      return messages.tooManyRequests ?? TOO_MANY_REQUESTS_MESSAGE
    case 503:
      return messages.busy ?? BUSY_MESSAGE
    case 408:
      return messages.timeout ?? TIMEOUT_MESSAGE
    default:
      return null
  }
}

/** A 409 from a settings route that stores a secret means the stored one cannot be decrypted. */
export function isSecretUnreadable(error: unknown): boolean {
  return error instanceof HttpErrorResponse && error.status === 409
}

/** What a settings form holding a secret tells the user about a failed save. */
export function secretFormFailureMessage(error: unknown, fallback: string): string {
  if (isSecretUnreadable(error)) {
    return SECRET_UNREADABLE_MESSAGE
  }
  return badRequestMessage(error) ?? overloadMessage(error) ?? fallback
}

/** Like `badRequestMessage`, plus a 409 (a name already taken is worth telling the user) and the overload statuses. */
export function rejectionMessage(error: unknown): string | null {
  return error instanceof HttpErrorResponse && (error.status === 400 || error.status === 409)
    ? bodyMessage(error)
    : overloadMessage(error)
}
