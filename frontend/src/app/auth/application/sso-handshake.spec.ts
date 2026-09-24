import { consumeSsoStart, discardSsoStart, markSsoStarted } from './sso-handshake'

describe('sso handshake', () => {
  afterEach(() => {
    sessionStorage.clear()
    vi.restoreAllMocks()
  })

  it('accepts a fresh start once, then refuses', () => {
    markSsoStarted('/x', 1_000)

    expect(consumeSsoStart(1_000 + 9 * 60 * 1000)).toEqual({ returnUrl: '/x' })
    expect(consumeSsoStart(1_000)).toBeNull()
  })

  it('refuses a start older than ten minutes', () => {
    markSsoStarted(null, 1_000)

    expect(consumeSsoStart(1_000 + 10 * 60 * 1000 + 1)).toBeNull()
  })

  it('refuses a start dated in the future', () => {
    markSsoStarted(null, 5_000)

    expect(consumeSsoStart(1_000)).toBeNull()
  })

  it('refuses garbage in the slot', () => {
    sessionStorage.setItem('artiferris_sso_pending', 'not json')

    expect(consumeSsoStart()).toBeNull()
  })

  it('does not throw when storage is unavailable', () => {
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('quota')
    })

    expect(() => markSsoStarted(null)).not.toThrow()
  })

  it('refuses a start that was discarded', () => {
    markSsoStarted('/x')

    discardSsoStart()

    expect(consumeSsoStart()).toBeNull()
  })

  it('does not throw when storage is unavailable while discarding', () => {
    vi.stubGlobal('sessionStorage', {
      removeItem: () => {
        throw new Error('blocked')
      },
    })

    expect(() => discardSsoStart()).not.toThrow()
    vi.unstubAllGlobals()
  })
})
