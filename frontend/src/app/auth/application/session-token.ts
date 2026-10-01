import { Injectable, signal } from '@angular/core'

const TOKEN_STORAGE_KEY = 'artiferris_token'

// The JWT lives in sessionStorage, not an HttpOnly cookie (accepted risk). That is only safe while
// the CSP forbids unsafe-inline and
// unsafe-eval and untrusted HTML reaches the DOM only through ReadmeView. A new innerHTML or
// bypassSecurityTrust* sink, a looser CSP
// or a compromised dependency means moving to an HttpOnly cookie with CSRF protection.
/** The session token. AuthService owns the login flow; this only holds the value. */
@Injectable({ providedIn: 'root' })
export class SessionToken {
  // With blocked storage the token lives in memory until reload.
  readonly value = signal<string | null>(readStored())

  set(token: string): void {
    this.value.set(token)
    try {
      sessionStorage.setItem(TOKEN_STORAGE_KEY, token)
    } catch {
      // Blocked storage: the token stays in memory.
    }
  }

  clear(): void {
    this.value.set(null)
    try {
      sessionStorage.removeItem(TOKEN_STORAGE_KEY)
    } catch {
      // Blocked storage: nothing to remove.
    }
  }
}

function readStored(): string | null {
  try {
    return sessionStorage.getItem(TOKEN_STORAGE_KEY)
  } catch {
    return null
  }
}
