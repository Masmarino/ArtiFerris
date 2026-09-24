import { HttpErrorResponse } from '@angular/common/http'
import { ComponentFixture, TestBed } from '@angular/core/testing'
import { ActivatedRoute, provideRouter } from '@angular/router'
import { NEVER, Observable, of, throwError } from 'rxjs'
import { AuthService } from '../../../auth/application/auth.service'
import { CatalogService } from '../application/catalog.service'
import { CatalogInfo, CatalogQuery, CatalogSearchResult } from '../domain/catalog.entity'
import { CATALOG_INFOS, catalogEntry, dockerEntry, searchResult } from '../testing/catalog.fixtures'
import { FakeActivatedRoute } from '../testing/fake-url'
import { ExplorerPage } from './explorer-page'

async function render(
  catalogs: () => Observable<CatalogInfo[]> = () => of(CATALOG_INFOS),
  neverSettles = false,
  options: {
    params?: Record<string, string>
    search?: (query: CatalogQuery) => Observable<CatalogSearchResult>
  } = {},
) {
  const route = new FakeActivatedRoute(options.params)
  const search = vi.fn(options.search ?? (() => of(searchResult([catalogEntry()]))))
  TestBed.configureTestingModule({
    providers: [
      provideRouter([]),
      { provide: ActivatedRoute, useValue: route },
      { provide: CatalogService, useValue: { catalogs, search } },
      { provide: AuthService, useValue: { isAuthenticated: () => false } },
    ],
  })
  const fixture = TestBed.createComponent(ExplorerPage)
  await settle(fixture, neverSettles)
  return { fixture, el: fixture.nativeElement as HTMLElement, search, route }
}

async function settle(fixture: ComponentFixture<unknown>, neverSettles = false) {
  fixture.detectChanges()
  if (neverSettles) {
    await new Promise((resolve) => setTimeout(resolve))
  } else {
    await fixture.whenStable()
  }
  fixture.detectChanges()
}

describe('ExplorerPage', () => {
  it('has the explorer heading', async () => {
    const { el } = await render()

    expect(el.querySelector('h1')!.textContent).toBe('Explorer les paquets publics')
  })

  it('shows one card per catalog with name, label and entry count, linking to the catalog page', async () => {
    const { el } = await render()

    const cards = Array.from(el.querySelectorAll<HTMLAnchorElement>('.explorer-page__catalog'))
    expect(cards.map((card) => card.getAttribute('href'))).toEqual([
      '/artiferris-npm',
      '/artiferris-docker',
    ])
    expect(cards[0].textContent).toContain('artiferris-npm')
    expect(cards[0].textContent).toContain('npm')
    expect(cards[0].textContent).toContain('12 paquets')
    expect(cards[1].textContent).toContain('1 image')
    expect(cards[1].textContent).not.toContain('1 images')
  })

  it('shows a spinner while the catalogs load, and the search regardless', async () => {
    const { el } = await render(() => NEVER, true)

    expect(el.querySelector('gbt-spinner')).not.toBeNull()
    expect(el.querySelector('app-catalog-search')).not.toBeNull()
  })

  it('reports a catalog loading failure and retries on demand', async () => {
    let attempt = 0
    const { fixture, el } = await render(() =>
      ++attempt === 1
        ? throwError(() => new HttpErrorResponse({ status: 500 }))
        : of(CATALOG_INFOS),
    )
    expect(el.querySelector('[role="alert"]')!.textContent).toContain(
      'Échec du chargement des catalogues.',
    )

    el.querySelector<HTMLButtonElement>('gbt-button button')!.click()
    await settle(fixture)

    expect(el.querySelector('[role="alert"]')).toBeNull()
    expect(el.querySelectorAll('.explorer-page__catalog')).toHaveLength(2)
  })

  it('says the catalog is busy when loading the catalogs answers a 503', async () => {
    const { el } = await render(() => throwError(() => new HttpErrorResponse({ status: 503 })))

    expect(el.querySelector('[role="alert"]')!.textContent).toContain(
      'Le catalogue est momentanément occupé',
    )
  })

  it('searches across all formats and offers every catalog as a filter', async () => {
    const { el, search } = await render()

    expect(search).toHaveBeenCalledWith(expect.objectContaining({ format: undefined }))
    const labels = Array.from(el.querySelectorAll('[aria-label="Format"] [role="radio"]')).map(
      (b) => b.textContent?.trim(),
    )
    expect(labels).toEqual(['Tous', 'npm', 'Docker'])
  })

  describe('popular this week', () => {
    const popular = (query: CatalogQuery) =>
      query.sort === 'popular'
        ? of(
            searchResult([
              catalogEntry({ name: 'left-pad', downloads_7d: 1500 }),
              dockerEntry({ downloads_7d: 3 }),
            ]),
          )
        : of(searchResult([catalogEntry()]))

    function popularSection(el: HTMLElement) {
      return el.querySelector('.explorer-page__popular')
    }

    it('lists the top 5 by weekly downloads above the recent listing', async () => {
      const { el, search } = await render(() => of(CATALOG_INFOS), false, { search: popular })

      expect(search).toHaveBeenCalledWith({ sort: 'popular', perPage: 5 })
      const section = popularSection(el)!
      expect(section.querySelector('h2')!.textContent).toContain('Populaires cette semaine')
      expect(Array.from(section.querySelectorAll('h3 a')).map((link) => link.textContent)).toEqual([
        'left-pad',
        'team/api',
      ])
      expect(section.textContent).toContain('1\u202f500 téléchargements cette semaine')
      expect(
        section.compareDocumentPosition(el.querySelector('app-catalog-search')!) &
          Node.DOCUMENT_POSITION_FOLLOWING,
      ).toBeTruthy()
    })

    it('is hidden when no entry was downloaded', async () => {
      const { el } = await render(() => of(CATALOG_INFOS), false, {
        search: () => of(searchResult([catalogEntry(), dockerEntry()])),
      })

      expect(popularSection(el)).toBeNull()
    })

    it('is hidden when the instance has nothing', async () => {
      const { el } = await render(() => of(CATALOG_INFOS), false, {
        search: () => of(searchResult([])),
      })

      expect(popularSection(el)).toBeNull()
    })

    it('shows a spinner while loading, apart from the search', async () => {
      const { el } = await render(() => of(CATALOG_INFOS), true, {
        search: (query) => (query.sort === 'popular' ? NEVER : of(searchResult([catalogEntry()]))),
      })

      expect(popularSection(el)!.querySelector('gbt-spinner')).not.toBeNull()
      expect(el.querySelector('app-catalog-search h3 a')!.textContent).toBe('hangar-demo')
    })

    it('reports its own failure without touching the search, and retries on demand', async () => {
      let attempt = 0
      const { fixture, el } = await render(() => of(CATALOG_INFOS), false, {
        search: (query) =>
          query.sort !== 'popular'
            ? of(searchResult([catalogEntry()]))
            : ++attempt === 1
              ? throwError(() => new HttpErrorResponse({ status: 500 }))
              : popular(query),
      })

      expect(popularSection(el)!.querySelector('[role="alert"]')!.textContent).toContain(
        'Échec du chargement des paquets populaires.',
      )
      expect(el.querySelector('app-catalog-search [role="alert"]')).toBeNull()
      expect(el.querySelector('app-catalog-search h3 a')!.textContent).toBe('hangar-demo')

      popularSection(el)!.querySelector<HTMLButtonElement>('gbt-button button')!.click()
      await settle(fixture)

      expect(popularSection(el)!.querySelector('[role="alert"]')).toBeNull()
      expect(popularSection(el)!.querySelectorAll('h3')).toHaveLength(2)
    })

    it('says the catalog is busy when the popular list answers a 503', async () => {
      const { el } = await render(() => of(CATALOG_INFOS), false, {
        search: (query) =>
          query.sort !== 'popular'
            ? of(searchResult([catalogEntry()]))
            : throwError(() => new HttpErrorResponse({ status: 503 })),
      })

      expect(popularSection(el)!.querySelector('[role="alert"]')!.textContent).toContain(
        'Le catalogue est momentanément occupé',
      )
    })

    it.each([
      ['a text search', { q: 'demo' }],
      ['a format filter', { format: 'npm' }],
      ['the popular sort', { sort: 'popular' }],
      ['a later page', { page: '2' }],
    ])('does not request or show the section during %s', async (_label, params) => {
      const { el, search } = await render(() => of(CATALOG_INFOS), false, {
        params,
        search: popular,
      })

      expect(search).not.toHaveBeenCalledWith(expect.objectContaining({ perPage: 5 }))
      expect(popularSection(el)).toBeNull()
    })

    it('follows the URL: leaves when a search starts and comes back with the landing', async () => {
      const { fixture, el, search, route } = await render(() => of(CATALOG_INFOS), false, {
        search: popular,
      })
      const popularRequests = () =>
        search.mock.calls.filter(([query]) => query.perPage === 5).length
      expect(popularRequests()).toBe(1)

      route.set({ q: 'demo' })
      await settle(fixture)
      expect(popularSection(el)).toBeNull()
      expect(popularRequests()).toBe(1)

      route.set({})
      await settle(fixture)
      expect(popularSection(el)).not.toBeNull()
      expect(popularRequests()).toBe(2)
    })

    it('still shows for a blank text or an unknown format, which the search ignores too', async () => {
      const { el } = await render(() => of(CATALOG_INFOS), false, {
        params: { q: '   ', format: 'helm' },
        search: popular,
      })

      expect(popularSection(el)).not.toBeNull()
    })
  })
})
