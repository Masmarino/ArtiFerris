import { ReloadEnv, isChunkLoadError, reloadOnce } from './chunk-reload'

function memoryEnv(now = 0): ReloadEnv & { assigned: string[]; clock: { t: number } } {
  const data = new Map<string, string>()
  const clock = { t: now }
  const assigned: string[] = []
  return {
    storage: { getItem: (k) => data.get(k) ?? null, setItem: (k, v) => void data.set(k, v) },
    assign: (url) => void assigned.push(url),
    now: () => clock.t,
    assigned,
    clock,
  }
}

describe('isChunkLoadError', () => {
  it('recognises the failure as each browser words it', () => {
    expect(
      isChunkLoadError(new TypeError('Failed to fetch dynamically imported module: /chunk-A.js')),
    ).toBe(true)
    expect(isChunkLoadError(new TypeError('Importing a module script failed.'))).toBe(true)
    expect(
      isChunkLoadError(new TypeError('error loading dynamically imported module: /chunk-A.js')),
    ).toBe(true)
  })

  it('leaves other errors alone', () => {
    expect(isChunkLoadError(new Error('Cannot match any routes'))).toBe(false)
    expect(isChunkLoadError(undefined)).toBe(false)
  })
})

describe('reloadOnce', () => {
  it('loads the address in full the first time', () => {
    const env = memoryEnv()
    expect(reloadOnce('/fr/ci/', env)).toBe(true)
    expect(env.assigned).toEqual(['/fr/ci/'])
  })

  it('does not loop on the same address, but tries again later or elsewhere', () => {
    const env = memoryEnv(1_000)
    reloadOnce('/fr/ci/', env)
    env.clock.t = 3_000
    expect(reloadOnce('/fr/ci/', env)).toBe(false)
    expect(reloadOnce('/fr/install/', env)).toBe(true)
    env.clock.t = 30_000
    expect(reloadOnce('/fr/install/', env)).toBe(true)
    expect(env.assigned).toEqual(['/fr/ci/', '/fr/install/', '/fr/install/'])
  })

  it('does nothing without storage, since it could not stop a loop', () => {
    const env = { ...memoryEnv(), storage: null }
    expect(reloadOnce('/fr/ci/', env)).toBe(false)
    const throwing = {
      ...memoryEnv(),
      storage: {
        getItem: () => {
          throw new Error('blocked')
        },
        setItem: () => undefined,
      },
    }
    expect(reloadOnce('/fr/ci/', throwing)).toBe(false)
    expect(throwing.assigned).toEqual([])
  })
})
