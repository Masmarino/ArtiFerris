import { Injectable, signal } from '@angular/core'

const TOKEN_STORAGE_KEY = 'artiferris_token'

// The JWT sits in sessionStorage rather than an HttpOnly cookie (accepted risk, M-19). That
// holds only while script-src stays free of unsafe-inline/unsafe-eval (pinned by the CSP test in
// artiferris-api's main.rs) and untrusted HTML reaches the DOM only through ReadmeView, which is
// sanitized server-side (ammonia) and again by Angular. A new innerHTML or bypassSecurityTrust*
// sink, a relaxed CSP or a compromised dependency means moving to an HttpOnly cookie + CSRF.
/** The session token. AuthService owns the login flow; this only holds the value. */
@Injectable({ providedIn: 'root' })
export class SessionToken {
  // With blocked storage the session lives in the signal only, until reload.
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
