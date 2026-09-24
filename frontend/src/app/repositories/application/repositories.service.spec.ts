import { TestBed } from '@angular/core/testing'
import { SessionToken } from '../../auth/application/session-token'
import { of, throwError } from 'rxjs'
import { RepositoriesService } from './repositories.service'
import { REPOSITORY_PORT, RepositoryPort } from './repository.port'

describe('RepositoriesService', () => {
  function setup(port: Partial<RepositoryPort>) {
    TestBed.configureTestingModule({ providers: [{ provide: REPOSITORY_PORT, useValue: port }] })
    return TestBed.inject(RepositoriesService)
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
    service.create('my-repo', 'npm', 'hosted', null).subscribe()
    service.list().subscribe()

    expect(list).toHaveBeenCalledTimes(2)
  })

  it('a mutation (delete) clears the cache so the next list() call re-fetches', () => {
    const list = vi.fn().mockReturnValue(of([]))
    const del = vi.fn().mockReturnValue(of(undefined))
    const service = setup({ list, delete: del })

    service.list().subscribe()
    service.delete('repo-1').subscribe()
    service.list().subscribe()

    expect(list).toHaveBeenCalledTimes(2)
  })

  it('delegates get() to the port', () => {
    const get = vi.fn().mockReturnValue(of({}))
    setup({ get }).get('repo-1')

    expect(get).toHaveBeenCalledWith('repo-1')
  })

  it('delegates getByOwner() to the port, with no caching', () => {
    const getByOwner = vi.fn().mockReturnValue(of({}))
    const service = setup({ getByOwner })

    service.getByOwner('alice', 'my-lib').subscribe()
    service.getByOwner('alice', 'my-lib').subscribe()

    expect(getByOwner).toHaveBeenCalledTimes(2)
    expect(getByOwner).toHaveBeenCalledWith('alice', 'my-lib')
  })

  it('delegates getByOrg() to the port, with no caching', () => {
    const getByOrg = vi.fn().mockReturnValue(of({}))
    const service = setup({ getByOrg })

    service.getByOrg('acme', 'my-lib').subscribe()
    service.getByOrg('acme', 'my-lib').subscribe()

    expect(getByOrg).toHaveBeenCalledTimes(2)
    expect(getByOrg).toHaveBeenCalledWith('acme', 'my-lib')
  })

  it('delegates create() to the port, defaulting options to an empty object', () => {
    const create = vi.fn().mockReturnValue(of({}))
    setup({ create }).create('my-repo', 'npm', 'hosted', null)

    expect(create).toHaveBeenCalledWith('my-repo', 'npm', 'hosted', null, {})
  })

  it('delegates delete() to the port', () => {
    const del = vi.fn().mockReturnValue(of(undefined))
    setup({ delete: del }).delete('repo-1')

    expect(del).toHaveBeenCalledWith('repo-1')
  })

  it('delegates setVisibility() to the port', () => {
    const setVisibility = vi.fn().mockReturnValue(of(undefined))
    setup({ setVisibility }).setVisibility('repo-1', true)

    expect(setVisibility).toHaveBeenCalledWith('repo-1', true)
  })

  it('a mutation (setVisibility) clears the cache so the next list() call re-fetches', () => {
    const list = vi.fn().mockReturnValue(of([]))
    const setVisibility = vi.fn().mockReturnValue(of(undefined))
    const service = setup({ list, setVisibility })

    service.list().subscribe()
    service.setVisibility('repo-1', true).subscribe()
    service.list().subscribe()

    expect(list).toHaveBeenCalledTimes(2)
  })

  it('delegates packages() to the port', () => {
    const packages = vi.fn().mockReturnValue(of({ format: 'npm', packages: [] }))
    setup({ packages }).packages('repo-1')

    expect(packages).toHaveBeenCalledWith('repo-1', undefined)
  })

  it('forwards the paging cursor of packages() to the port', () => {
    const packages = vi.fn().mockReturnValue(of({ format: 'npm', packages: [] }))
    setup({ packages }).packages('repo-1', 'left-pad')

    expect(packages).toHaveBeenCalledWith('repo-1', 'left-pad')
  })

  it('delegates scanDockerImage() to the port', () => {
    const scanDockerImage = vi.fn().mockReturnValue(of({ scanned_at: '', vulnerabilities: [] }))
    setup({ scanDockerImage }).scanDockerImage('repo-1', 'my-image', 'latest')

    expect(scanDockerImage).toHaveBeenCalledWith('repo-1', 'my-image', 'latest')
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
