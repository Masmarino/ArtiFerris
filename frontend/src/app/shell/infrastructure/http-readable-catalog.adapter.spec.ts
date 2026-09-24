import { TestBed } from '@angular/core/testing'
import { HttpErrorResponse, provideHttpClient } from '@angular/common/http'
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing'
import {
  proxiedEntry,
  readableDockerEntry,
  readableEntry,
  readableSearchResult,
} from '../testing/readable-catalog.fixtures'
import { HttpReadableCatalogAdapter } from './http-readable-catalog.adapter'

describe('HttpReadableCatalogAdapter', () => {
  function setup() {
    TestBed.configureTestingModule({
      providers: [provideHttpClient(), provideHttpClientTesting(), HttpReadableCatalogAdapter],
    })
    return {
      adapter: TestBed.inject(HttpReadableCatalogAdapter),
      httpMock: TestBed.inject(HttpTestingController),
    }
  }

  afterEach(() => TestBed.inject(HttpTestingController).verify())

  it('sends no parameter at all for an empty query', () => {
    const { adapter, httpMock } = setup()
    adapter.search({}).subscribe()
    const req = httpMock.expectOne('/api/search')
    expect(req.request.method).toBe('GET')
    expect(req.request.params.keys()).toEqual([])
    req.flush(readableSearchResult([]))
  })

  it('omits blank text', () => {
    const { adapter, httpMock } = setup()
    adapter.search({ q: '   ' }).subscribe()
    const req = httpMock.expectOne('/api/search')
    expect(req.request.params.keys()).toEqual([])
    req.flush(readableSearchResult([]))
  })

  it('maps every field to its query parameter, trimming the text', () => {
    const { adapter, httpMock } = setup()
    adapter
      .search({ q: '  demo ', format: 'docker', sort: 'updated', page: 3, perPage: 10 })
      .subscribe()
    const req = httpMock.expectOne((r) => r.url === '/api/search')
    expect(req.request.params.get('q')).toBe('demo')
    expect(req.request.params.get('format')).toBe('docker')
    expect(req.request.params.get('sort')).toBe('updated')
    expect(req.request.params.get('page')).toBe('3')
    expect(req.request.params.get('per_page')).toBe('10')
    req.flush(readableSearchResult([]))
  })

  it('encodes characters that would otherwise change the query string', () => {
    const { adapter, httpMock } = setup()
    adapter.search({ q: 'c++ & co #1' }).subscribe()
    const req = httpMock.expectOne((r) => r.url === '/api/search')
    expect(req.request.urlWithParams).toBe('/api/search?q=c%2B%2B%20%26%20co%20%231')
    req.flush(readableSearchResult([]))
  })

  it('returns the response as is, including the repository of each entry', () => {
    const { adapter, httpMock } = setup()
    const response = readableSearchResult([readableEntry(), readableDockerEntry(), proxiedEntry()])
    let result
    adapter.search({ q: 'a' }).subscribe((r) => (result = r))
    httpMock.expectOne((r) => r.url === '/api/search').flush(response)
    expect(result).toEqual(response)
  })

  it.each([400, 401, 429, 500])('lets a %i through as an error', (status) => {
    const { adapter, httpMock } = setup()
    let error: unknown
    adapter.search({ q: 'demo' }).subscribe({ error: (e) => (error = e) })
    httpMock
      .expectOne((r) => r.url === '/api/search')
      .flush({ error: 'nope' }, { status, statusText: 'Error' })
    expect((error as HttpErrorResponse).status).toBe(status)
  })
})
