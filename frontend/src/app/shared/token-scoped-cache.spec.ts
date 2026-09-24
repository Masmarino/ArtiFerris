import { TestBed } from '@angular/core/testing'
import { Injector, runInInjectionContext } from '@angular/core'
import { Subject, of, throwError } from 'rxjs'
import { SessionToken } from '../auth/application/session-token'
import { TokenScopedCache } from './token-scoped-cache'

describe('TokenScopedCache', () => {
  function setup(load: () => ReturnType<typeof of>) {
    const session = TestBed.inject(SessionToken)
    const cache = runInInjectionContext(TestBed.inject(Injector), () => new TokenScopedCache(load))
    return { cache, session }
  }

  beforeEach(() => sessionStorage.clear())

  it('shares one request between callers under the same token', () => {
    const load = vi.fn().mockReturnValue(of(['a']))
    const { cache, session } = setup(load)
    session.set('t1')

    cache.get().subscribe()
    cache.get().subscribe()

    expect(load).toHaveBeenCalledTimes(1)
  })

  it('refetches once the token differs, even before any effect has run', () => {
    const load = vi
      .fn()
      .mockReturnValueOnce(of(['a']))
      .mockReturnValueOnce(of(['b']))
    const { cache, session } = setup(load)
    session.set('t1')
    cache.get().subscribe()

    session.set('t2')
    let seen: unknown
    cache.get().subscribe((value) => (seen = value))

    expect(seen).toEqual(['b'])
  })

  it('drops the entry eagerly when the token changes', () => {
    const load = vi.fn().mockReturnValue(of(['a']))
    const { cache, session } = setup(load)
    session.set('t1')
    TestBed.tick()
    cache.get().subscribe()

    session.clear()
    TestBed.tick()
    session.set('t1')
    cache.get().subscribe()

    expect(load).toHaveBeenCalledTimes(2)
  })

  it('does not cache a failure', () => {
    const load = vi
      .fn()
      .mockReturnValueOnce(throwError(() => new Error('boom')))
      .mockReturnValue(of([]))
    const { cache } = setup(load)

    cache.get().subscribe({ error: () => undefined })
    cache.get().subscribe()

    expect(load).toHaveBeenCalledTimes(2)
  })

  it('a late failure of an old request does not evict the newer entry', () => {
    const first = new Subject<string[]>()
    const load = vi
      .fn()
      .mockReturnValueOnce(first)
      .mockReturnValue(of(['fresh']))
    const { cache, session } = setup(load)
    session.set('t1')
    cache.get().subscribe({ error: () => undefined })
    session.set('t2')
    cache.get().subscribe()

    first.error(new Error('late'))
    cache.get().subscribe()

    expect(load).toHaveBeenCalledTimes(2)
  })

  it('forceRefresh bypasses the entry', () => {
    const load = vi.fn().mockReturnValue(of([]))
    const { cache } = setup(load)

    cache.get().subscribe()
    cache.get(true).subscribe()

    expect(load).toHaveBeenCalledTimes(2)
  })
})
