/**
 * A page's code is a separate chunk. When it cannot be fetched (a network blip, or a page kept open across a deploy
 * that removed the old chunks), the router gives up and hydration clears the prerendered page. A full load of the
 * same address brings the current HTML and its chunks; it is tried once per address so it can never loop.
 */

const KEY = 'chunk-reload'
const WINDOW_MS = 10_000

// Chrome, Safari and Firefox word the failure differently.
const CHUNK_ERROR =
  /Failed to fetch dynamically imported module|Importing a module script failed|error loading dynamically imported module/i

export function isChunkLoadError(error: unknown): boolean {
  const message = error instanceof Error ? error.message : String(error ?? '')
  return CHUNK_ERROR.test(message)
}

export interface ReloadEnv {
  storage: Pick<Storage, 'getItem' | 'setItem'> | null
  assign: (url: string) => void
  now: () => number
}

/** Loads `url` in full, unless that was already tried for the same address in the last few seconds. */
export function reloadOnce(url: string, env: ReloadEnv): boolean {
  // Without storage (private mode, blocked) nothing could stop a loop, so the page is left as it is.
  if (!env.storage) return false
  try {
    const last = JSON.parse(env.storage.getItem(KEY) ?? 'null') as {
      url: string
      at: number
    } | null
    if (last && last.url === url && env.now() - last.at < WINDOW_MS) return false
    env.storage.setItem(KEY, JSON.stringify({ url, at: env.now() }))
  } catch {
    return false
  }
  env.assign(url)
  return true
}

export function browserReloadEnv(): ReloadEnv {
  return {
    storage: sessionStorageOrNull(),
    assign: (url) => window.location.assign(url),
    now: () => Date.now(),
  }
}

function sessionStorageOrNull(): Storage | null {
  try {
    return window.sessionStorage
  } catch {
    return null
  }
}
