import { TestBed } from '@angular/core/testing'
import { of } from 'rxjs'
import {
  CATALOG_INFOS,
  catalogSuggestion,
  ownerSummary,
  searchResult,
} from '../testing/catalog.fixtures'
import { CatalogService } from './catalog.service'
import { PUBLIC_CATALOG_PORT, PublicCatalogPort } from './public-catalog.port'

describe('CatalogService', () => {
  function setup(port: Partial<PublicCatalogPort>) {
    TestBed.configureTestingModule({
      providers: [{ provide: PUBLIC_CATALOG_PORT, useValue: port }],
    })
    return TestBed.inject(CatalogService)
  }

  it('delegates catalogs() to the port', () => {
    let result
    setup({ catalogs: () => of(CATALOG_INFOS) })
      .catalogs()
      .subscribe((r) => (result = r))

    expect(result).toEqual(CATALOG_INFOS)
  })

  it('delegates search() to the port with the query untouched', () => {
    const search = vi.fn().mockReturnValue(of(searchResult([])))
    const query = { q: 'demo', format: 'npm' as const, page: 2 }

    setup({ search }).search(query).subscribe()

    expect(search).toHaveBeenCalledWith(query)
  })

  it('delegates suggest() to the port', () => {
    const suggest = vi.fn().mockReturnValue(of([catalogSuggestion()]))
    let result
    setup({ suggest })
      .suggest('demo')
      .subscribe((r) => (result = r))

    expect(suggest).toHaveBeenCalledWith('demo', undefined)
    expect(result).toEqual([catalogSuggestion()])
  })

  it('passes the filters and the limit on to the port', () => {
    const suggest = vi.fn().mockReturnValue(of([]))
    const options = {
      format: 'npm' as const,
      owner: { kind: 'personal' as const, slug: 'alice' },
      limit: 20,
    }

    setup({ suggest }).suggest('demo', options).subscribe()

    expect(suggest).toHaveBeenCalledWith('demo', options)
  })

  it('delegates owner() to the port', () => {
    const owner = vi.fn().mockReturnValue(of(ownerSummary()))
    let result
    setup({ owner })
      .owner('personal', 'admin')
      .subscribe((r) => (result = r))

    expect(owner).toHaveBeenCalledWith('personal', 'admin')
    expect(result).toEqual(ownerSummary())
  })

  it('passes a missing owner (null) through', () => {
    let result
    setup({ owner: () => of(null) })
      .owner('organization', 'ghost')
      .subscribe((r) => (result = r))

    expect(result).toBeNull()
  })
})
