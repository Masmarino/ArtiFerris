import { TestBed } from '@angular/core/testing'
import { HttpErrorResponse, provideHttpClient } from '@angular/common/http'
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing'
import {
  CATALOG_INFOS,
  catalogSuggestion,
  ownerSummary,
  searchResult,
} from '../testing/catalog.fixtures'
import { HttpPublicCatalogAdapter } from './http-public-catalog.adapter'

describe('HttpPublicCatalogAdapter', () => {
  function setup() {
    TestBed.configureTestingModule({
      providers: [provideHttpClient(), provideHttpClientTesting(), HttpPublicCatalogAdapter],
    })
    return {
      adapter: TestBed.inject(HttpPublicCatalogAdapter),
      httpMock: TestBed.inject(HttpTestingController),
    }
  }

  afterEach(() => TestBed.inject(HttpTestingController).verify())

  it('lists the catalogs', () => {
    const { adapter, httpMock } = setup()

    let result
    adapter.catalogs().subscribe((r) => (result = r))
    const req = httpMock.expectOne('/api/public/catalogs')
    expect(req.request.method).toBe('GET')
    req.flush(CATALOG_INFOS)

    expect(result).toEqual(CATALOG_INFOS)
  })

  it('sends no parameter at all for an empty query', () => {
    const { adapter, httpMock } = setup()

    adapter.search({}).subscribe()

    const req = httpMock.expectOne('/api/public/search')
    expect(req.request.params.keys()).toEqual([])
    req.flush(searchResult([]))
  })

  it('omits empty and blank parameters', () => {
    const { adapter, httpMock } = setup()

    adapter.search({ q: '   ', format: undefined, sort: undefined }).subscribe()

    const req = httpMock.expectOne('/api/public/search')
    expect(req.request.params.keys()).toEqual([])
    req.flush(searchResult([]))
  })

  it('maps every field to its query parameter, trimming the text', () => {
    const { adapter, httpMock } = setup()

    adapter
      .search({
        q: '  demo ',
        format: 'docker',
        owner: { kind: 'organization', slug: 'acme' },
        sort: 'updated',
        page: 3,
        perPage: 10,
      })
      .subscribe()

    const req = httpMock.expectOne((r) => r.url === '/api/public/search')
    expect(req.request.params.get('q')).toBe('demo')
    expect(req.request.params.get('format')).toBe('docker')
    expect(req.request.params.get('owner')).toBe('organization:acme')
    expect(req.request.params.get('sort')).toBe('updated')
    expect(req.request.params.get('page')).toBe('3')
    expect(req.request.params.get('per_page')).toBe('10')
    req.flush(searchResult([]))
  })

  it('sends the popular sort, with or without text', () => {
    const { adapter, httpMock } = setup()

    adapter.search({ sort: 'popular', perPage: 5 }).subscribe()

    const req = httpMock.expectOne((r) => r.url === '/api/public/search')
    expect(req.request.params.get('sort')).toBe('popular')
    expect(req.request.params.get('per_page')).toBe('5')
    expect(req.request.params.has('q')).toBe(false)
    req.flush(searchResult([]))
  })

  describe('suggest', () => {
    it('gets the suggestions for the text', () => {
      const { adapter, httpMock } = setup()

      let result
      adapter.suggest('demo').subscribe((r) => (result = r))
      const req = httpMock.expectOne('/api/public/suggest?q=demo')
      expect(req.request.method).toBe('GET')
      req.flush([catalogSuggestion()])

      expect(result).toEqual([catalogSuggestion()])
    })

    it('sends the limit only when one is given', () => {
      const { adapter, httpMock } = setup()

      adapter.suggest('demo', { limit: 20 }).subscribe()

      httpMock.expectOne('/api/public/suggest?q=demo&limit=20').flush([])
    })

    it('sends the format and the owner so the server filters', () => {
      const { adapter, httpMock } = setup()

      adapter
        .suggest('demo', { format: 'docker', owner: { kind: 'organization', slug: 'acme' } })
        .subscribe()

      const req = httpMock.expectOne((r) => r.url === '/api/public/suggest')
      expect(req.request.params.get('format')).toBe('docker')
      expect(req.request.params.get('owner')).toBe('organization:acme')
      expect(req.request.params.has('limit')).toBe(false)
      req.flush([])
    })

    it('sends a personal owner as personal:<username>', () => {
      const { adapter, httpMock } = setup()

      adapter.suggest('demo', { owner: { kind: 'personal', slug: 'alice' } }).subscribe()

      const req = httpMock.expectOne((r) => r.url === '/api/public/suggest')
      expect(req.request.params.get('owner')).toBe('personal:alice')
      req.flush([])
    })

    it('leaves null filters out', () => {
      const { adapter, httpMock } = setup()

      adapter.suggest('demo', { format: null, owner: null }).subscribe()

      httpMock.expectOne('/api/public/suggest?q=demo').flush([])
    })

    it('trims the text', () => {
      const { adapter, httpMock } = setup()

      adapter.suggest('  demo ').subscribe()

      httpMock.expectOne('/api/public/suggest?q=demo').flush([])
    })

    it('encodes characters that would otherwise change the query string', () => {
      const { adapter, httpMock } = setup()

      adapter.suggest('c++ & co #1').subscribe()

      const req = httpMock.expectOne((r) => r.url === '/api/public/suggest')
      expect(req.request.params.get('q')).toBe('c++ & co #1')
      expect(req.request.urlWithParams).toBe('/api/public/suggest?q=c%2B%2B%20%26%20co%20%231')
      req.flush([])
    })

    it.each([400, 429, 500])('lets a %i through as an error', (status) => {
      const { adapter, httpMock } = setup()

      let error: unknown
      adapter.suggest('demo').subscribe({ error: (e) => (error = e) })
      httpMock.expectOne('/api/public/suggest?q=demo').flush({}, { status, statusText: 'Error' })

      expect((error as HttpErrorResponse).status).toBe(status)
    })
  })

  describe('owner', () => {
    it('gets the summary of a personal owner', () => {
      const { adapter, httpMock } = setup()

      let result
      adapter.owner('personal', 'admin').subscribe((r) => (result = r))
      const req = httpMock.expectOne('/api/public/owners/personal/admin')
      expect(req.request.method).toBe('GET')
      req.flush(ownerSummary())

      expect(result).toEqual(ownerSummary())
    })

    it('encodes the slug into a single path segment', () => {
      const { adapter, httpMock } = setup()

      adapter.owner('organization', 'a/b').subscribe()

      httpMock.expectOne('/api/public/owners/organization/a%2Fb').flush(ownerSummary())
    })

    it('emits null for a 404', () => {
      const { adapter, httpMock } = setup()

      let result: unknown = 'unset'
      adapter.owner('personal', 'ghost').subscribe((r) => (result = r))
      httpMock
        .expectOne('/api/public/owners/personal/ghost')
        .flush({ error: 'not found' }, { status: 404, statusText: 'Not Found' })

      expect(result).toBeNull()
    })

    it.each([429, 500])('lets a %i through as an error', (status) => {
      const { adapter, httpMock } = setup()

      let error: unknown
      adapter.owner('personal', 'admin').subscribe({ error: (e) => (error = e) })
      httpMock
        .expectOne('/api/public/owners/personal/admin')
        .flush({}, { status, statusText: 'Error' })

      expect((error as HttpErrorResponse).status).toBe(status)
    })
  })
})
