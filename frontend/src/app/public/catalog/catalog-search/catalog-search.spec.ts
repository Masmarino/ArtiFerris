import { HttpErrorResponse } from '@angular/common/http'
import { ComponentFixture, TestBed } from '@angular/core/testing'
import { ActivatedRoute, Router, provideRouter } from '@angular/router'
import { NEVER, Observable, of, throwError } from 'rxjs'
import { CatalogService } from '../application/catalog.service'
import {
  CatalogFormat,
  CatalogInfo,
  CatalogQuery,
  CatalogSearchResult,
  CatalogSuggestion,
  OwnerRef,
  SuggestOptions,
} from '../domain/catalog.entity'
import {
  CATALOG_INFOS,
  catalogEntry,
  catalogSuggestion,
  dockerEntry,
  dockerSuggestion,
  searchResult,
} from '../testing/catalog.fixtures'
import { FakeActivatedRoute, navigateFake } from '../testing/fake-url'
import { CatalogSearch } from './catalog-search'

interface Options {
  /** For a search that never answers: `whenStable` would wait for it forever. */
  neverSettles?: boolean
  params?: Record<string, string>
  search?: (query: CatalogQuery) => Observable<CatalogSearchResult>
  suggest?: (text: string, options?: SuggestOptions) => Observable<CatalogSuggestion[]>
  format?: CatalogFormat | null
  owner?: OwnerRef | null
  catalogs?: CatalogInfo[]
}

async function render(options: Options = {}) {
  const route = new FakeActivatedRoute(options.params)
  const search = vi.fn(options.search ?? (() => of(searchResult([catalogEntry()]))))
  const suggest = vi.fn(options.suggest ?? (() => of([])))

  TestBed.configureTestingModule({
    providers: [
      provideRouter([]),
      { provide: ActivatedRoute, useValue: route },
      { provide: CatalogService, useValue: { search, suggest } },
    ],
  })
  const navigate = vi
    .spyOn(TestBed.inject(Router), 'navigate')
    .mockImplementation((_commands, extras) => navigateFake(route, extras))
  const fixture = TestBed.createComponent(CatalogSearch)
  if (options.format !== undefined) {
    fixture.componentRef.setInput('format', options.format)
  }
  if (options.owner !== undefined) {
    fixture.componentRef.setInput('owner', options.owner)
  }
  if (options.catalogs) {
    fixture.componentRef.setInput('catalogs', options.catalogs)
  }
  if (options.neverSettles) {
    fixture.detectChanges()
    await new Promise((resolve) => setTimeout(resolve))
    fixture.detectChanges()
  } else {
    await settle(fixture)
  }
  return { fixture, el: fixture.nativeElement as HTMLElement, navigate, search, suggest, route }
}

async function settle(fixture: ComponentFixture<unknown>) {
  fixture.detectChanges()
  await fixture.whenStable()
  fixture.detectChanges()
}

function typeText(el: HTMLElement, value: string) {
  const input = el.querySelector('input')!
  input.value = value
  input.dispatchEvent(new Event('input'))
}

function submit(el: HTMLElement) {
  el.querySelector('form')!.dispatchEvent(new Event('submit'))
}

function radio(el: HTMLElement, name: string): HTMLButtonElement {
  return Array.from(el.querySelectorAll<HTMLButtonElement>('[role="radio"]')).find(
    (button) => button.textContent?.trim() === name,
  )!
}

function button(el: HTMLElement, name: string): HTMLButtonElement {
  return Array.from(el.querySelectorAll<HTMLButtonElement>('gbt-button button')).find((b) =>
    b.textContent?.includes(name),
  )!
}

function queryParamsOf(navigate: ReturnType<typeof vi.fn>, call = -1): Record<string, unknown> {
  const calls = navigate.mock.calls
  return calls[call < 0 ? calls.length + call : call][1].queryParams
}

describe('CatalogSearch', () => {
  afterEach(() => vi.useRealTimers())

  describe('URL-driven state', () => {
    it('searches the most recent items when the URL has no parameters', async () => {
      const { search } = await render()

      expect(search).toHaveBeenCalledTimes(1)
      expect(search).toHaveBeenCalledWith({
        q: undefined,
        format: undefined,
        sort: undefined,
        page: 1,
      })
    })

    it('turns q, format, sort and page from the URL into the query', async () => {
      const { search } = await render({
        params: { q: ' demo ', format: 'docker', sort: 'updated', page: '3' },
      })

      expect(search).toHaveBeenCalledWith({
        q: 'demo',
        format: 'docker',
        sort: 'updated',
        page: 3,
      })
    })

    it('falls back to defaults for garbage values', async () => {
      const { search } = await render({ params: { format: 'helm', sort: 'newest', page: '-2' } })

      expect(search).toHaveBeenCalledWith({
        q: undefined,
        format: undefined,
        sort: undefined,
        page: 1,
      })
    })

    it('requests the popular sort from the URL, with or without text', async () => {
      const bare = await render({ params: { sort: 'popular' } })
      expect(bare.search).toHaveBeenCalledWith(expect.objectContaining({ sort: 'popular' }))
      expect(bare.search.mock.calls[0][0].q).toBeUndefined()

      TestBed.resetTestingModule()
      const withText = await render({ params: { q: 'demo', sort: 'popular' } })
      expect(withText.search).toHaveBeenCalledWith(
        expect.objectContaining({ q: 'demo', sort: 'popular' }),
      )
    })

    it('caps the text at 100 characters', async () => {
      const { search } = await render({ params: { q: 'x'.repeat(150) } })

      expect(search.mock.calls[0][0].q).toHaveLength(100)
    })

    it('fills the search box from the URL and follows it when Back changes it', async () => {
      const { fixture, el, route } = await render({ params: { q: 'demo' } })
      expect(el.querySelector('input')!.value).toBe('demo')

      route.set({ q: 'other' })
      await settle(fixture)

      expect(el.querySelector('input')!.value).toBe('other')
    })

    it('titles the list after the query, or "Récemment mis à jour" without text', async () => {
      const withText = await render({ params: { q: 'demo' } })
      expect(withText.el.querySelector('h2')!.textContent).toContain('Résultats pour « demo »')
      TestBed.resetTestingModule()

      const withoutText = await render()
      expect(withoutText.el.querySelector('h2')!.textContent).toContain('Récemment mis à jour')
    })
  })

  describe('typing', () => {
    it('waits 300 ms before pushing the text to the URL, without adding a history entry', async () => {
      const { el, navigate } = await render()
      vi.useFakeTimers()

      typeText(el, 'demo')
      vi.advanceTimersByTime(299)
      expect(navigate).not.toHaveBeenCalled()

      vi.advanceTimersByTime(1)
      expect(navigate).toHaveBeenCalledTimes(1)
      expect(navigate.mock.calls[0][1]).toMatchObject({
        queryParams: { q: 'demo', page: null },
        queryParamsHandling: 'merge',
        replaceUrl: true,
      })
    })

    it('restarts the delay on every keystroke and only pushes the last text', async () => {
      const { el, navigate } = await render()
      vi.useFakeTimers()

      typeText(el, 'd')
      vi.advanceTimersByTime(200)
      typeText(el, 'de')
      vi.advanceTimersByTime(200)
      typeText(el, 'dem')
      vi.advanceTimersByTime(300)

      expect(navigate).toHaveBeenCalledTimes(1)
      expect(queryParamsOf(navigate)).toEqual({ q: 'dem', page: null })
    })

    it('clears the parameter when the box is emptied', async () => {
      const { el, navigate } = await render({ params: { q: 'demo' } })
      vi.useFakeTimers()

      typeText(el, '  ')
      vi.advanceTimersByTime(300)

      expect(queryParamsOf(navigate)).toEqual({ q: null, page: null })
    })

    it('searches immediately on Enter, and drops the pending delayed search', async () => {
      const { el, navigate } = await render()
      vi.useFakeTimers()

      typeText(el, 'demo')
      submit(el)

      expect(navigate).toHaveBeenCalledTimes(1)
      expect(navigate.mock.calls[0][1]).toMatchObject({
        queryParams: { q: 'demo', page: null },
        replaceUrl: false,
      })

      vi.advanceTimersByTime(1000)
      expect(navigate).toHaveBeenCalledTimes(1)
    })

    it('does not overwrite what the user is typing when the URL catches up', async () => {
      const { fixture, el, route } = await render()
      vi.useFakeTimers()

      typeText(el, 'dem')
      route.set({ q: 'de' })
      fixture.detectChanges()

      expect(el.querySelector('input')!.value).toBe('dem')
    })

    it('runs the search once the URL follows', async () => {
      const { fixture, el, search } = await render()
      vi.useFakeTimers()
      typeText(el, 'demo')
      vi.advanceTimersByTime(300)
      vi.useRealTimers()

      await settle(fixture)

      expect(search).toHaveBeenLastCalledWith(expect.objectContaining({ q: 'demo', page: 1 }))
    })
  })

  describe('filters', () => {
    it('offers Tous plus one option per catalog, and marks the current format', async () => {
      const { el } = await render({ params: { format: 'npm' }, catalogs: CATALOG_INFOS })

      const labels = Array.from(el.querySelectorAll('[role="radio"]')).map((b) =>
        b.textContent?.trim(),
      )
      expect(labels).toEqual(['Tous', 'npm', 'Docker', 'Pertinence', 'Récents', 'Populaires'])
      expect(radio(el, 'npm').getAttribute('aria-checked')).toBe('true')
    })

    it('falls back to the built-in catalog list before the catalogs are loaded', async () => {
      const { el } = await render()

      expect(radio(el, 'npm')).toBeTruthy()
      expect(radio(el, 'Docker')).toBeTruthy()
    })

    it('picking a format resets the page and adds the format to the URL', async () => {
      const { el, navigate } = await render({ params: { page: '4' } })

      radio(el, 'npm').click()

      expect(queryParamsOf(navigate)).toEqual({ format: 'npm', page: null })
    })

    it('picking Tous removes the format', async () => {
      const { el, navigate } = await render({ params: { format: 'npm' } })

      radio(el, 'Tous').click()

      expect(queryParamsOf(navigate)).toEqual({ format: null, page: null })
    })

    it('picking a sort resets the page', async () => {
      const { el, navigate } = await render({ params: { q: 'demo', page: '2' } })

      radio(el, 'Récents').click()

      expect(queryParamsOf(navigate)).toEqual({ sort: 'updated', page: null })
    })

    it('picking Populaires puts sort=popular in the URL and resets the page', async () => {
      const { el, navigate } = await render({ params: { page: '3' } })

      radio(el, 'Populaires').click()

      expect(queryParamsOf(navigate)).toEqual({ sort: 'popular', page: null })
    })

    it('round-trips sort=popular: the option is checked and the search follows the URL', async () => {
      const { fixture, el, route, search } = await render()
      radio(el, 'Populaires').click()
      await settle(fixture)

      expect(route.current['sort']).toBe('popular')
      expect(radio(el, 'Populaires').getAttribute('aria-checked')).toBe('true')
      expect(search).toHaveBeenLastCalledWith(expect.objectContaining({ sort: 'popular', page: 1 }))
    })

    it('keeps Populaires selected when there is no text, with Pertinence still disabled', async () => {
      const { el } = await render({ params: { sort: 'popular' } })

      expect(radio(el, 'Populaires').getAttribute('aria-checked')).toBe('true')
      expect(radio(el, 'Pertinence').disabled).toBe(true)
    })

    it('titles the popular listing without text, and keeps the results heading with text', async () => {
      const bare = await render({ params: { sort: 'popular' } })
      expect(bare.el.querySelector('h2')!.textContent).toContain('Populaires cette semaine')

      TestBed.resetTestingModule()
      const withText = await render({ params: { q: 'demo', sort: 'popular' } })
      expect(withText.el.querySelector('h2')!.textContent).toContain('Résultats pour « demo »')
    })

    it('keeps the locked format and owner in a popular search', async () => {
      const { search } = await render({
        format: 'npm',
        owner: { kind: 'organization', slug: 'acme' },
        params: { sort: 'popular' },
      })

      expect(search).toHaveBeenCalledWith(
        expect.objectContaining({
          format: 'npm',
          owner: { kind: 'organization', slug: 'acme' },
          sort: 'popular',
        }),
      )
    })

    it('shows Récents and disables Pertinence when there is no text', async () => {
      const { el } = await render()

      expect(radio(el, 'Récents').getAttribute('aria-checked')).toBe('true')
      expect(radio(el, 'Pertinence').disabled).toBe(true)
    })

    it('defaults to Pertinence once there is text', async () => {
      const { el } = await render({ params: { q: 'demo' } })

      expect(radio(el, 'Pertinence').getAttribute('aria-checked')).toBe('true')
      expect(radio(el, 'Pertinence').disabled).toBe(false)
    })
  })

  describe('locked format', () => {
    it('hides the format filter and always searches that format', async () => {
      const { el, search } = await render({ format: 'npm' })

      expect(el.querySelector('[aria-label="Format"]')).toBeNull()
      expect(el.querySelector('[aria-label="Trier par"]')).not.toBeNull()
      expect(search).toHaveBeenCalledWith(expect.objectContaining({ format: 'npm' }))
    })

    it('ignores a format in the URL that differs from the lock', async () => {
      const { search } = await render({ format: 'npm', params: { format: 'docker' } })

      expect(search).toHaveBeenCalledWith(expect.objectContaining({ format: 'npm' }))
    })

    it('keeps the lock when the search is cleared', async () => {
      const { fixture, el, navigate } = await render({
        format: 'npm',
        params: { q: 'nothing' },
        search: () => of(searchResult([])),
      })
      await settle(fixture)

      button(el, 'Effacer la recherche').click()

      expect(queryParamsOf(navigate)).toEqual({ q: null, page: null })
    })
  })

  describe('locked owner', () => {
    const acme: OwnerRef = { kind: 'organization', slug: 'acme' }

    it('adds the owner to every search, next to the URL parameters', async () => {
      const { search } = await render({ owner: acme, params: { q: 'demo', format: 'docker' } })

      expect(search).toHaveBeenCalledWith({
        q: 'demo',
        format: 'docker',
        owner: acme,
        sort: undefined,
        page: 1,
      })
    })

    it('keeps the format filter available', async () => {
      const { el } = await render({ owner: acme })

      expect(el.querySelector('[aria-label="Format"]')).not.toBeNull()
    })

    it('never writes the owner into the URL', async () => {
      const { el, navigate } = await render({ owner: acme })

      radio(el, 'Docker').click()
      submit(el)

      expect(navigate.mock.calls.length).toBeGreaterThan(0)
      navigate.mock.calls.forEach((_, index) =>
        expect(queryParamsOf(navigate, index)).not.toHaveProperty('owner'),
      )
    })

    it('ignores an owner parameter in the URL', async () => {
      const { search } = await render({ params: { owner: 'personal:someone' } })

      expect(search).toHaveBeenCalledWith(expect.objectContaining({ owner: undefined }))
    })

    it('says the owner has nothing to show, and can drop the format filter', async () => {
      const { fixture, el, navigate } = await render({
        owner: acme,
        params: { format: 'docker' },
        search: () => of(searchResult([])),
      })

      const empty = el.querySelector('gbt-empty-state')!
      expect(empty.textContent).toContain("Ce propriétaire n'a rien de public")
      expect(empty.textContent).not.toContain('Rendez un dépôt public')

      button(el, 'Tous les formats').click()
      await settle(fixture)

      expect(queryParamsOf(navigate)).toEqual({ format: null, page: null })
    })

    it('offers no format reset when there is no filter to drop', async () => {
      const { el } = await render({ owner: acme, search: () => of(searchResult([])) })

      expect(button(el, 'Tous les formats')).toBeUndefined()
    })
  })

  describe('pagination', () => {
    const threePages = (page: number) => () =>
      of(searchResult([catalogEntry()], { total: 45, per_page: 20, page }))

    it('shows "Page x sur y" and no pagination when everything fits on one page', async () => {
      const single = await render()
      expect(single.el.querySelector('nav[aria-label="Pagination"]')).toBeNull()
      TestBed.resetTestingModule()

      const { el } = await render({ params: { page: '2' }, search: threePages(2) })
      expect(el.querySelector('.catalog-search__page')!.textContent).toBe('Page 2 sur 3')
    })

    it('goes to the next and the previous page', async () => {
      const { el, navigate } = await render({ params: { page: '2' }, search: threePages(2) })

      button(el, 'Suivant').click()
      expect(queryParamsOf(navigate)).toEqual({ page: 3 })

      button(el, 'Précédent').click()
      expect(queryParamsOf(navigate)).toEqual({ page: null })
    })

    it('disables Précédent on the first page', async () => {
      const { el } = await render({ search: threePages(1) })

      expect(button(el, 'Précédent').disabled).toBe(true)
      expect(button(el, 'Suivant').disabled).toBe(false)
    })

    it('disables Suivant on the last page', async () => {
      const { el } = await render({ params: { page: '3' }, search: threePages(3) })

      expect(button(el, 'Suivant').disabled).toBe(true)
      expect(button(el, 'Précédent').disabled).toBe(false)
    })

    it('moves focus to the results heading on a page change', async () => {
      const { fixture, el } = await render({ params: { page: '2' }, search: threePages(2) })
      document.body.appendChild(el)

      button(el, 'Suivant').click()
      fixture.detectChanges()

      expect(document.activeElement).toBe(el.querySelector('h2'))
      el.remove()
    })

    it('falls back to the last page when the URL asks for one beyond the end', async () => {
      const { navigate } = await render({
        params: { page: '9' },
        search: () => of(searchResult([], { total: 45, per_page: 20, page: 9 })),
      })

      expect(navigate.mock.calls[0][1]).toMatchObject({
        queryParams: { page: 3 },
        replaceUrl: true,
      })
    })
  })

  describe('suggestions', () => {
    const SUGGESTIONS = [
      catalogSuggestion({ name: 'left-pad' }),
      dockerSuggestion(),
      catalogSuggestion({
        name: 'left-acme',
        owner: { kind: 'organization', slug: 'acme', display_name: 'Acme Corp' },
      }),
    ]

    function suggested(el: HTMLElement): string[] {
      return Array.from(el.querySelectorAll('[role="option"] .suggest__name')).map(
        (name) => name.textContent!,
      )
    }

    async function typeSuggest(rendered: Awaited<ReturnType<typeof render>>, text = 'left') {
      vi.useFakeTimers()
      typeText(rendered.el, text)
      vi.advanceTimersByTime(200)
      rendered.fixture.detectChanges()
    }

    function key(el: HTMLElement, key: string) {
      el.querySelector('input')!.dispatchEvent(
        new KeyboardEvent('keydown', { key, cancelable: true }),
      )
    }

    it('suggests names while typing, before the URL search has run', async () => {
      const rendered = await render({ suggest: () => of(SUGGESTIONS) })

      await typeSuggest(rendered)

      expect(suggested(rendered.el)).toEqual(['left-pad', 'team/api', 'left-acme'])
      expect(rendered.suggest).toHaveBeenCalledWith('left', { format: null, owner: null })
      expect(rendered.navigate).not.toHaveBeenCalled()
    })

    it('has the search box labelled and exposed as a combobox', async () => {
      const { el } = await render()

      const input = el.querySelector<HTMLInputElement>('input[role="combobox"]')!
      expect(el.querySelector(`label[for="${input.id}"]`)!.textContent).toBe('Rechercher un paquet')
      expect(input.placeholder).toBe('Nom, description ou mot-clé')
    })

    it('does not narrow the suggestions to a format chosen in the URL filters', async () => {
      const rendered = await render({
        params: { format: 'docker' },
        suggest: () => of(SUGGESTIONS),
      })

      await typeSuggest(rendered)

      expect(suggested(rendered.el)).toHaveLength(3)
      expect(rendered.suggest).toHaveBeenCalledWith('left', { format: null, owner: null })
    })

    it('asks the server for the locked format', async () => {
      const rendered = await render({ format: 'docker', suggest: () => of([SUGGESTIONS[1]]) })

      await typeSuggest(rendered)

      expect(rendered.suggest).toHaveBeenCalledWith('left', { format: 'docker', owner: null })
      expect(suggested(rendered.el)).toEqual(['team/api'])
    })

    it('asks the server for the locked owner', async () => {
      const rendered = await render({
        owner: { kind: 'organization', slug: 'acme' },
        suggest: () => of([SUGGESTIONS[1], SUGGESTIONS[2]]),
      })

      await typeSuggest(rendered)

      expect(rendered.suggest).toHaveBeenCalledWith('left', {
        format: null,
        owner: { kind: 'organization', slug: 'acme' },
      })
      expect(suggested(rendered.el)).toEqual(['team/api', 'left-acme'])
    })

    it('asks the server for the locked format and owner together', async () => {
      const rendered = await render({
        format: 'npm',
        owner: { kind: 'organization', slug: 'acme' },
        suggest: () => of([SUGGESTIONS[2]]),
      })

      await typeSuggest(rendered)

      expect(rendered.suggest).toHaveBeenCalledWith('left', {
        format: 'npm',
        owner: { kind: 'organization', slug: 'acme' },
      })
      expect(suggested(rendered.el)).toEqual(['left-acme'])
    })

    it('searches immediately on Enter without a highlighted suggestion', async () => {
      const rendered = await render({ suggest: () => of(SUGGESTIONS) })
      await typeSuggest(rendered)

      key(rendered.el, 'Enter')

      expect(rendered.navigate).toHaveBeenCalledTimes(1)
      expect(rendered.navigate.mock.calls[0][1]).toMatchObject({
        queryParams: { q: 'left', page: null },
        replaceUrl: false,
      })
      vi.advanceTimersByTime(1000)
      expect(rendered.navigate).toHaveBeenCalledTimes(1)
    })

    it('opens the package page on Enter with a highlighted suggestion, without searching', async () => {
      const rendered = await render({ suggest: () => of(SUGGESTIONS) })
      await typeSuggest(rendered)

      key(rendered.el, 'ArrowDown')
      key(rendered.el, 'Enter')

      expect(rendered.navigate).toHaveBeenCalledTimes(1)
      expect(rendered.navigate.mock.calls[0][0]).toEqual([
        '/@admin',
        'test-npm',
        'packages',
        'npm',
        'left-pad',
      ])
    })

    it('still searches from the Rechercher button', async () => {
      const rendered = await render()
      vi.useFakeTimers()
      typeText(rendered.el, 'demo')

      submit(rendered.el)

      expect(rendered.navigate).toHaveBeenCalledTimes(1)
      expect(queryParamsOf(rendered.navigate)).toEqual({ q: 'demo', page: null })
    })
  })

  describe('approximate results', () => {
    const exact = catalogEntry({ name: 'left-pad', match_kind: 'prefix' })
    const otherExact = catalogEntry({ name: 'left', match_kind: 'exact' })
    const fuzzy = catalogEntry({ name: 'lefft-pad', match_kind: 'fuzzy' })
    const fuzzyDocker = dockerEntry({ match_kind: 'fuzzy' })

    function names(container: Element): string[] {
      return Array.from(container.querySelectorAll('.result-card__name')).map((n) =>
        n.textContent!.trim(),
      )
    }

    function groups(el: HTMLElement) {
      const lists = el.querySelectorAll<HTMLElement>('.catalog-search__results')
      const heading = el.querySelector('.catalog-search__approximate-heading')
      const hint = el.querySelector('.catalog-search__hint')
      return { lists, heading, hint }
    }

    it('lists fuzzy results after the others under "Résultats approchants"', async () => {
      const { el } = await render({
        params: { q: 'left' },
        search: () => of(searchResult([exact, fuzzy, otherExact, fuzzyDocker])),
      })

      const { lists, heading } = groups(el)
      expect(lists).toHaveLength(2)
      expect(names(lists[0])).toEqual(['left-pad', 'left'])
      expect(heading!.textContent).toBe('Résultats approchants')
      expect(names(lists[1])).toEqual(['lefft-pad', 'team/api'])
      expect(
        lists[0].compareDocumentPosition(heading!) & Node.DOCUMENT_POSITION_FOLLOWING,
      ).toBeTruthy()
    })

    it('shows no heading and no hint without a fuzzy result', async () => {
      const { el } = await render({
        params: { q: 'left' },
        search: () => of(searchResult([exact, otherExact])),
      })

      const { lists, heading, hint } = groups(el)
      expect(lists).toHaveLength(1)
      expect(heading).toBeNull()
      expect(hint).toBeNull()
    })

    it('does not add a hint when exact and fuzzy results are mixed', async () => {
      const { el } = await render({
        params: { q: 'left' },
        search: () => of(searchResult([exact, fuzzy])),
      })

      expect(groups(el).hint).toBeNull()
    })

    it('says there is no exact result when every result is fuzzy', async () => {
      const { el } = await render({
        params: { q: 'lefft' },
        search: () => of(searchResult([fuzzy, fuzzyDocker])),
      })

      const { lists, heading, hint } = groups(el)
      expect(hint!.textContent!.trim()).toBe(
        'Aucun résultat exact — voici des résultats approchants.',
      )
      expect(lists).toHaveLength(1)
      expect(names(lists[0])).toEqual(['lefft-pad', 'team/api'])
      expect(
        hint!.compareDocumentPosition(heading!) & Node.DOCUMENT_POSITION_FOLLOWING,
      ).toBeTruthy()
    })

    it('keeps the total and the pagination of the whole result set', async () => {
      const { el } = await render({
        params: { q: 'left' },
        search: () => of(searchResult([exact, fuzzy], { total: 45, per_page: 20 })),
      })

      expect(el.querySelector('.catalog-search__count')!.textContent).toBe('45 résultats')
      expect(el.querySelector('.catalog-search__page')!.textContent).toBe('Page 1 sur 3')
    })

    it('treats results without a match kind as regular results', async () => {
      const { el } = await render({
        search: () => of(searchResult([catalogEntry({ match_kind: null })])),
      })

      expect(groups(el).heading).toBeNull()
      expect(el.querySelectorAll('app-catalog-result-card')).toHaveLength(1)
    })
  })

  describe('states', () => {
    it('renders one card per result', async () => {
      const { el } = await render({
        search: () => of(searchResult([catalogEntry(), dockerEntry()])),
      })

      expect(el.querySelectorAll('app-catalog-result-card')).toHaveLength(2)
    })

    it('shows a spinner while the search is running', async () => {
      const { el } = await render({ search: () => NEVER, neverSettles: true })

      expect(el.querySelector('gbt-spinner')).not.toBeNull()
      expect(el.querySelector('app-catalog-result-card')).toBeNull()
    })

    it('announces the result count politely, singular and plural', async () => {
      const many = await render({
        search: () => of(searchResult([catalogEntry()], { total: 42 })),
      })
      expect(many.el.querySelector('.catalog-search__count')!.textContent).toBe('42 résultats')
      TestBed.resetTestingModule()

      const one = await render({ search: () => of(searchResult([catalogEntry()])) })
      expect(one.el.querySelector('.catalog-search__count')!.textContent).toBe('1 résultat')
    })

    it('shows an error with a retry button that runs the search again', async () => {
      let attempt = 0
      const { fixture, el, search } = await render({
        search: () =>
          ++attempt === 1
            ? throwError(() => new HttpErrorResponse({ status: 500 }))
            : of(searchResult([catalogEntry()])),
      })
      expect(el.querySelector('[role="alert"]')!.textContent).toContain('La recherche a échoué')
      expect(el.querySelector('app-catalog-result-card')).toBeNull()

      button(el, 'Réessayer').click()
      await settle(fixture)

      expect(search).toHaveBeenCalledTimes(2)
      expect(el.querySelector('[role="alert"]')).toBeNull()
      expect(el.querySelectorAll('app-catalog-result-card')).toHaveLength(1)
    })

    it('tells the visitor to slow down on a 429', async () => {
      const { el } = await render({
        search: () => throwError(() => new HttpErrorResponse({ status: 429 })),
      })

      expect(el.querySelector('[role="alert"]')!.textContent).toContain('Trop de requêtes')
    })

    it('says the catalog is busy on a 503', async () => {
      const { el } = await render({
        search: () => throwError(() => new HttpErrorResponse({ status: 503 })),
      })

      expect(el.querySelector('[role="alert"]')!.textContent).toContain(
        'Le catalogue est momentanément occupé',
      )
    })

    it('shows a no-results state that suggests another search, and can clear the query', async () => {
      const { fixture, el, navigate } = await render({
        params: { q: 'nothing', format: 'npm' },
        search: () => of(searchResult([])),
      })
      expect(el.querySelector('gbt-empty-state')!.textContent).toContain('Aucun résultat')
      expect(el.querySelector('gbt-empty-state')!.textContent).toContain('un autre nom')
      expect(el.querySelector('.catalog-search__count')!.textContent).toBe('Aucun résultat')

      button(el, 'Effacer la recherche').click()
      await settle(fixture)

      expect(queryParamsOf(navigate)).toEqual({ q: null, format: null, page: null })
    })

    it('shows the instance-empty state when nothing is public and nothing was searched', async () => {
      const { el } = await render({ search: () => of(searchResult([])) })

      const empty = el.querySelector('gbt-empty-state')!
      expect(empty.textContent).toContain("Rien à explorer pour l'instant")
      expect(empty.textContent).not.toContain('Effacer la recherche')
    })
  })
})
