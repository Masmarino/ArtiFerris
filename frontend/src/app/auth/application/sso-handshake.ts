const STORAGE_KEY = 'artiferris_sso_pending'
const MAX_AGE_MS = 10 * 60 * 1000

interface Pending {
  startedAt: number
  returnUrl: string | null
}

export function markSsoStarted(returnUrl: string | null, now = Date.now()): void {
  try {
    const pending: Pending = { startedAt: now, returnUrl }
    sessionStorage.setItem(STORAGE_KEY, JSON.stringify(pending))
  } catch {
    // Without storage the return trip is refused.
  }
}

export function discardSsoStart(): void {
  try {
    sessionStorage.removeItem(STORAGE_KEY)
  } catch {
    // Blocked storage: nothing to remove.
  }
}

/** One-shot: true only if this tab started an SSO login in the last 10 minutes. */
export function consumeSsoStart(now = Date.now()): { returnUrl: string | null } | null {
  let raw: string | null
  try {
    raw = sessionStorage.getItem(STORAGE_KEY)
    sessionStorage.removeItem(STORAGE_KEY)
  } catch {
    return null
  }
  if (raw === null) {
    return null
  }
  try {
    const pending = JSON.parse(raw) as Partial<Pending>
    const age = now - Number(pending.startedAt)
    if (!Number.isFinite(age) || age < 0 || age > MAX_AGE_MS) {
      return null
    }
    return { returnUrl: typeof pending.returnUrl === 'string' ? pending.returnUrl : null }
  } catch {
    return null
  }
}
