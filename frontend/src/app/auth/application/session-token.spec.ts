import { TestBed } from '@angular/core/testing'
import { SessionToken } from './session-token'

describe('SessionToken', () => {
  afterEach(() => {
    vi.unstubAllGlobals()
    sessionStorage.clear()
    vi.restoreAllMocks()
  })

  function blockStorage() {
    const blocked = () => {
      throw new DOMException('blocked', 'SecurityError')
    }
    vi.stubGlobal('sessionStorage', {
      getItem: blocked,
      setItem: blocked,
      removeItem: blocked,
    })
    expect(() => sessionStorage.getItem('probe')).toThrow()
  }

  it('restores the stored token', () => {
    sessionStorage.setItem('artiferris_token', 'stored')

    expect(TestBed.inject(SessionToken).value()).toBe('stored')
  })

  it('persists and clears the token in sessionStorage', () => {
    const session = TestBed.inject(SessionToken)

    session.set('jwt')
    expect(sessionStorage.getItem('artiferris_token')).toBe('jwt')

    session.clear()
    expect(sessionStorage.getItem('artiferris_token')).toBeNull()
    expect(session.value()).toBeNull()
  })

  it('can be created when storage is blocked, with no token', () => {
    blockStorage()

    expect(TestBed.inject(SessionToken).value()).toBeNull()
  })

  it('keeps the token in memory when storage refuses writes', () => {
    blockStorage()
    const session = TestBed.inject(SessionToken)

    session.set('jwt')
    expect(session.value()).toBe('jwt')

    session.clear()
    expect(session.value()).toBeNull()
  })
})
