import { TestBed } from '@angular/core/testing'
import { of } from 'rxjs'
import { readableEntry, readableSearchResult } from '../testing/readable-catalog.fixtures'
import { READABLE_CATALOG_PORT, ReadableCatalogPort } from './readable-catalog.port'
import { ReadableCatalogService } from './readable-catalog.service'

describe('ReadableCatalogService', () => {
  it('delegates the search to the port', () => {
    const result = readableSearchResult([readableEntry()])
    const port: ReadableCatalogPort = { search: vi.fn(() => of(result)) }
    TestBed.configureTestingModule({
      providers: [{ provide: READABLE_CATALOG_PORT, useValue: port }],
    })

    let received
    TestBed.inject(ReadableCatalogService)
      .search({ q: 'left', perPage: 5 })
      .subscribe((r) => (received = r))

    expect(port.search).toHaveBeenCalledWith({ q: 'left', perPage: 5 })
    expect(received).toBe(result)
  })
})
