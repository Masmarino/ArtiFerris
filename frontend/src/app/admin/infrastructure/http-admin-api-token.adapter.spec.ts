import { TestBed } from '@angular/core/testing'
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing'
import { provideHttpClient } from '@angular/common/http'
import { HttpAdminApiTokenAdapter } from './http-admin-api-token.adapter'

describe('HttpAdminApiTokenAdapter', () => {
  function setup() {
    TestBed.configureTestingModule({
      providers: [provideHttpClient(), provideHttpClientTesting(), HttpAdminApiTokenAdapter],
    })
    return {
      adapter: TestBed.inject(HttpAdminApiTokenAdapter),
      httpMock: TestBed.inject(HttpTestingController),
    }
  }

  it('fetches tokens across every user, asking for the 500-row page explicitly', () => {
    const { adapter, httpMock } = setup()

    adapter.list().subscribe((tokens) => expect(tokens[0].username).toBe('florian'))
    httpMock.expectOne('/api/admin/tokens?limit=500').flush([
      {
        id: 't1',
        user_id: 'u1',
        username: 'florian',
        label: 'laptop',
        created_at: '2026-01-01T00:00:00Z',
        last_used_at: null,
        revoked_at: null,
      },
    ])
  })

  it('scopes the list to an organization and still sets the limit', () => {
    const { adapter, httpMock } = setup()

    adapter.list('org-1').subscribe()
    httpMock.expectOne('/api/admin/tokens?organization_id=org-1&limit=500').flush([])
  })

  it('revokes a token by id', () => {
    const { adapter, httpMock } = setup()

    adapter.revoke('t1').subscribe()
    const req = httpMock.expectOne('/api/admin/tokens/t1')
    expect(req.request.method).toBe('DELETE')
    req.flush(null)
  })
})
