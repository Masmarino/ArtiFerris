import { TestBed } from '@angular/core/testing'
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing'
import { provideHttpClient } from '@angular/common/http'
import { HttpPersonalRepositoryAdapter } from './http-personal-repository.adapter'

describe('HttpPersonalRepositoryAdapter', () => {
  function setup() {
    TestBed.configureTestingModule({
      providers: [provideHttpClient(), provideHttpClientTesting(), HttpPersonalRepositoryAdapter],
    })
    return {
      adapter: TestBed.inject(HttpPersonalRepositoryAdapter),
      httpMock: TestBed.inject(HttpTestingController),
    }
  }

  it('resolves true when GET /api/me/repository succeeds', () => {
    const { adapter, httpMock } = setup()

    let result: boolean | undefined
    adapter.checkReserved().subscribe((value) => (result = value))

    httpMock.expectOne('/api/me/repository').flush(null)
    expect(result).toBe(true)
  })

  it('maps a 404 from GET /api/me/repository to false instead of an error', () => {
    const { adapter, httpMock } = setup()

    let result: boolean | undefined
    adapter.checkReserved().subscribe((value) => (result = value))

    httpMock
      .expectOne('/api/me/repository')
      .flush('not found', { status: 404, statusText: 'Not Found' })
    expect(result).toBe(false)
  })

  it('propagates a non-404 error from GET /api/me/repository', () => {
    const { adapter, httpMock } = setup()

    let error: unknown
    adapter.checkReserved().subscribe({ error: (err: unknown) => (error = err) })

    httpMock
      .expectOne('/api/me/repository')
      .flush('boom', { status: 500, statusText: 'Internal Server Error' })
    expect(error).toBeTruthy()
  })

  it('sends a POST to /api/me/repository to reserve the namespace', () => {
    const { adapter, httpMock } = setup()

    adapter.reserve().subscribe()

    const req = httpMock.expectOne('/api/me/repository')
    expect(req.request.method).toBe('POST')
    req.flush(null)
  })

  it("fetches the caller's projects from /api/me/repository/projects", () => {
    const { adapter, httpMock } = setup()

    let result: unknown
    adapter.listMyProjects().subscribe((value) => (result = value))

    httpMock.expectOne('/api/me/repository/projects').flush([
      {
        id: 'repo-1',
        name: 'my-project',
        format: 'npm',
        repo_type: 'hosted',
        remote_url: null,
        remote_credentials_set: false,
        group_members: [],
        quota_bytes: null,
        retention_keep_last_n: null,
        my_role: 'admin',
        organization_id: 'org-1',
        owner_name: 'florian',
        owner_is_personal: true,
      },
    ])

    expect(result).toEqual([expect.objectContaining({ id: 'repo-1', name: 'my-project' })])
  })

  it('creates a project with the expected payload', () => {
    const { adapter, httpMock } = setup()

    adapter.createProject('my-project', 'npm', 'hosted').subscribe()

    const req = httpMock.expectOne('/api/me/repository/projects')
    expect(req.request.method).toBe('POST')
    expect(req.request.body).toEqual({
      name: 'my-project',
      format: 'npm',
      repo_type: 'hosted',
    })
    req.flush({
      id: 'repo-1',
      name: 'my-project',
      format: 'npm',
      repo_type: 'hosted',
      remote_url: null,
      remote_credentials_set: false,
      group_members: [],
      quota_bytes: null,
      retention_keep_last_n: null,
      my_role: 'admin',
      organization_id: 'org-1',
      owner_name: 'florian',
      owner_is_personal: true,
    })
  })
})
