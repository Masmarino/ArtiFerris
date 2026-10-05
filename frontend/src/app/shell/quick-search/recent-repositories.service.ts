import { Injectable, signal } from '@angular/core'

const MAX_RECENT = 5

/**
 * The repositories an account last opened in this browser, newest first, offered by the quick search before anything
 * is typed. Kept per account so a shared browser doesn't show one person's repositories to the next. Storage can be
 * missing or full (private windows): the list then lives as long as the page.
 */
@Injectable({ providedIn: 'root' })
export class RecentRepositoriesService {
  // Bumped on every change, so a computed that reads `list()` follows writes made through `remember()`.
  private readonly version = signal(0)
  private readonly fallback = new Map<string, string[]>()

  /** Repository ids, newest first. */
  list(account: string | null): string[] {
    this.version()
    return account ? this.read(account) : []
  }

  remember(account: string | null, repositoryId: string): void {
    if (!account || !repositoryId) {
      return
    }
    const next = [repositoryId, ...this.read(account).filter((id) => id !== repositoryId)].slice(
      0,
      MAX_RECENT,
    )
    this.fallback.set(account, next)
    try {
      localStorage.setItem(storageKey(account), JSON.stringify(next))
    } catch {
      // Storage unavailable: the in-memory copy above is enough for this page.
    }
    this.version.update((v) => v + 1)
  }

  private read(account: string): string[] {
    try {
      const raw = localStorage.getItem(storageKey(account))
      if (raw !== null) {
        const parsed: unknown = JSON.parse(raw)
        if (Array.isArray(parsed)) {
          return parsed
            .filter((id): id is string => typeof id === 'string' && id !== '')
            .slice(0, MAX_RECENT)
        }
      }
    } catch {
      // Unreadable or corrupt: fall back to what this page remembered.
    }
    return this.fallback.get(account) ?? []
  }
}

function storageKey(account: string): string {
  return `artiferris.recent-repositories.${account}`
}
