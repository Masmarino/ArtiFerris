import { TestBed } from '@angular/core/testing'
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing'
import { provideHttpClient } from '@angular/common/http'
import { HttpAuditAdapter } from './http-audit.adapter'

describe('HttpAuditAdapter', () => {
  it('omits empty filter fields from the request params', () => {
    TestBed.configureTestingModule({
      providers: [provideHttpClient(), provideHttpClientTesting(), HttpAuditAdapter],
    })
    const adapter = TestBed.inject(HttpAuditAdapter)
    const httpMock = TestBed.inject(HttpTestingController)

    adapter.query({ aggregate_type: 'Security' }).subscribe()

    const req = httpMock.expectOne((r) => r.url === '/api/audit/events')
    expect(req.request.params.get('aggregate_type')).toBe('Security')
    expect(req.request.params.has('actor_id')).toBe(false)
    req.flush({ entries: [], next_cursor: null })
  })

  it('sends the cursor and the page size as query params and returns the page as is', () => {
    TestBed.configureTestingModule({
      providers: [provideHttpClient(), provideHttpClientTesting(), HttpAuditAdapter],
    })
    const adapter = TestBed.inject(HttpAuditAdapter)
    const httpMock = TestBed.inject(HttpTestingController)
    let page: unknown

    adapter.query({ cursor: 'abc', limit: 50 }).subscribe((p) => (page = p))

    const req = httpMock.expectOne((r) => r.url === '/api/audit/events')
    expect(req.request.params.get('cursor')).toBe('abc')
    expect(req.request.params.get('limit')).toBe('50')
    req.flush({ entries: [], next_cursor: 'def' })
    expect(page).toEqual({ entries: [], next_cursor: 'def' })
  })

  it('unlocks a username with a DELETE on its own path segment', () => {
    TestBed.configureTestingModule({
      providers: [provideHttpClient(), provideHttpClientTesting(), HttpAuditAdapter],
    })
    const adapter = TestBed.inject(HttpAuditAdapter)
    const httpMock = TestBed.inject(HttpTestingController)

    adapter.unlockUsername('a/b c').subscribe()

    const req = httpMock.expectOne('/api/admin/login-throttle/a%2Fb%20c')
    expect(req.request.method).toBe('DELETE')
    req.flush(null, { status: 204, statusText: 'No Content' })
  })
})
