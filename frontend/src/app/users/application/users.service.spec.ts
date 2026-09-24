import { TestBed } from '@angular/core/testing'
import { SessionToken } from '../../auth/application/session-token'
import { of, throwError } from 'rxjs'
import { UsersService } from './users.service'
import { USER_PORT, UserPort } from './user.port'

describe('UsersService', () => {
  function setup(port: Partial<UserPort>) {
    TestBed.configureTestingModule({ providers: [{ provide: USER_PORT, useValue: port }] })
    return TestBed.inject(UsersService)
  }

  it('delegates list() to the port', () => {
    const list = vi.fn().mockReturnValue(of([]))
    setup({ list }).list()

    expect(list).toHaveBeenCalled()
  })

  it('shares one cached list() across multiple callers instead of re-fetching', () => {
    const list = vi.fn().mockReturnValue(of([]))
    const service = setup({ list })

    service.list().subscribe()
    service.list().subscribe()

    expect(list).toHaveBeenCalledTimes(1)
  })

  it('does not permanently cache a failed list() — a later call retries', () => {
    const list = vi
      .fn()
      .mockReturnValueOnce(throwError(() => new Error('boom')))
      .mockReturnValue(of([]))
    const service = setup({ list })

    service.list().subscribe({ error: () => undefined })
    service.list().subscribe()

    expect(list).toHaveBeenCalledTimes(2)
  })

  it('a mutation (create) clears the cache so the next list() call re-fetches', () => {
    const list = vi.fn().mockReturnValue(of([]))
    const create = vi.fn().mockReturnValue(of({}))
    const service = setup({ list, create })

    service.list().subscribe()
    service.create('newuser', 'newuser@example.com', false).subscribe()
    service.list().subscribe()

    expect(list).toHaveBeenCalledTimes(2)
  })

  it('a mutation (setSuperAdmin) clears the cache so the next list() call re-fetches', () => {
    const list = vi.fn().mockReturnValue(of([]))
    const setSuperAdmin = vi.fn().mockReturnValue(of(undefined))
    const service = setup({ list, setSuperAdmin })

    service.list().subscribe()
    service.setSuperAdmin('user-1', true).subscribe()
    service.list().subscribe()

    expect(list).toHaveBeenCalledTimes(2)
  })

  it('resendInvitation() does not clear the cached list (it does not change list-visible fields)', () => {
    const list = vi.fn().mockReturnValue(of([]))
    const resendInvitation = vi.fn().mockReturnValue(of(undefined))
    const service = setup({ list, resendInvitation })

    service.list().subscribe()
    service.resendInvitation('user-1').subscribe()
    service.list().subscribe()

    expect(list).toHaveBeenCalledTimes(1)
  })

  it('delegates create() to the port', () => {
    const create = vi.fn().mockReturnValue(of({}))
    setup({ create }).create('newuser', 'newuser@example.com', false)

    expect(create).toHaveBeenCalledWith('newuser', 'newuser@example.com', false)
  })

  it('delegates delete() to the port', () => {
    const del = vi.fn().mockReturnValue(of(undefined))
    setup({ delete: del }).delete('user-1')

    expect(del).toHaveBeenCalledWith('user-1')
  })

  it('delegates setSuperAdmin() to the port', () => {
    const setSuperAdmin = vi.fn().mockReturnValue(of(undefined))
    setup({ setSuperAdmin }).setSuperAdmin('user-1', true)

    expect(setSuperAdmin).toHaveBeenCalledWith('user-1', true)
  })

  it('delegates resendInvitation() to the port', () => {
    const resendInvitation = vi.fn().mockReturnValue(of(undefined))
    setup({ resendInvitation }).resendInvitation('user-1')

    expect(resendInvitation).toHaveBeenCalledWith('user-1')
  })

  it("does not hand the previous user's list to the next user signed in on the same tab", () => {
    const list = vi
      .fn()
      .mockReturnValueOnce(of([{ id: 'alice-item' }]))
      .mockReturnValueOnce(of([{ id: 'bob-item' }]))
    const service = setup({ list })
    const session = TestBed.inject(SessionToken)

    session.set('alice-token')
    service.list().subscribe()
    session.set('bob-token')
    let seen: unknown
    service.list().subscribe((items) => (seen = items))

    expect(list).toHaveBeenCalledTimes(2)
    expect(seen).toEqual([{ id: 'bob-item' }])
  })

  it('refetches after logout instead of replaying the cached list', () => {
    const list = vi.fn().mockReturnValue(of([]))
    const service = setup({ list })
    const session = TestBed.inject(SessionToken)

    session.set('alice-token')
    service.list().subscribe()
    session.clear()
    service.list().subscribe()

    expect(list).toHaveBeenCalledTimes(2)
  })
})
