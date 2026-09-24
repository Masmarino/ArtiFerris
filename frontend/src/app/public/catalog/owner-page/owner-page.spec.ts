import { HttpErrorResponse } from '@angular/common/http'
import { ComponentFixture, TestBed } from '@angular/core/testing'
import { ActivatedRoute, convertToParamMap, provideRouter } from '@angular/router'
import { BehaviorSubject, NEVER, Observable, of, throwError } from 'rxjs'
import { AuthService } from '../../../auth/application/auth.service'
import { PageTitleService } from '../../../shell/page-title.service'
import { CatalogService } from '../application/catalog.service'
import { OwnerSummary } from '../domain/catalog.entity'
import { catalogEntry, ownerSummary, searchResult } from '../testing/catalog.fixtures'
import { OwnerPage } from './owner-page'

interface Options {
  params?: Record<string, string>
  owner?: (kind: string, slug: string) => Observable<OwnerSummary | null>
  neverSettles?: boolean
}

async function render(options: Options = {}) {
  const params = new BehaviorSubject(convertToParamMap(options.params ?? { username: 'admin' }))
  const owner = vi.fn(options.owner ?? (() => of(ownerSummary())))
  const search = vi.fn().mockReturnValue(of(searchResult([catalogEntry()])))
  TestBed.configureTestingModule({
    providers: [
      provideRouter([]),
      {
        provide: ActivatedRoute,
        useValue: { paramMap: params, queryParamMap: of(convertToParamMap({})) },
      },
      { provide: CatalogService, useValue: { owner, search } },
      { provide: AuthService, useValue: { isAuthenticated: () => false } },
    ],
  })
  const fixture = TestBed.createComponent(OwnerPage)
  await settle(fixture, options.neverSettles)
  return { fixture, el: fixture.nativeElement as HTMLElement, owner, search, params }
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

function button(el: HTMLElement, name: string): HTMLButtonElement {
  return Array.from(el.querySelectorAll<HTMLButtonElement>('gbt-button button')).find((b) =>
    b.textContent?.includes(name),
  )!
}

describe('OwnerPage', () => {
  describe('personal owner', () => {
    it('asks for the owner behind @username', async () => {
      const { owner } = await render({ params: { username: 'alice' } })

      expect(owner).toHaveBeenCalledWith('personal', 'alice')
    })

    it('shows the display name, the @username and the counts', async () => {
      const { el } = await render({
        owner: () => of(ownerSummary({ slug: 'alice', display_name: 'Alice Martin' })),
        params: { username: 'alice' },
      })

      expect(el.querySelector('h1')!.textContent).toBe('Alice Martin')
      expect(el.querySelector('.owner-page__header')!.textContent).toContain('@alice')
      expect(el.querySelector('.owner-page__counts')!.textContent).toBe(
        '2 dépôts · 3 paquets · 4 images',
      )
    })

    it('searches only that owner, from the summary the server confirmed', async () => {
      const { search, el } = await render({ params: { username: 'Admin' } })

      expect(search).toHaveBeenCalledWith(
        expect.objectContaining({ owner: { kind: 'personal', slug: 'admin' } }),
      )
      expect(el.querySelectorAll('app-catalog-result-card')).toHaveLength(1)
    })

    it('keeps the format filter available', async () => {
      const { el } = await render()

      expect(el.querySelector('[aria-label="Format"]')).not.toBeNull()
    })
  })

  describe('organization', () => {
    const acme = ownerSummary({ kind: 'organization', slug: 'acme', display_name: 'Acme Corp' })

    it('asks for the organization behind /o/:slug', async () => {
      const { owner } = await render({ params: { slug: 'acme' }, owner: () => of(acme) })

      expect(owner).toHaveBeenCalledWith('organization', 'acme')
    })

    it('shows the display name and no @handle', async () => {
      const { el } = await render({ params: { slug: 'acme' }, owner: () => of(acme) })

      expect(el.querySelector('h1')!.textContent).toBe('Acme Corp')
      expect(el.querySelector('.owner-page__header')!.textContent).not.toContain('@')
    })

    it('searches only that organization', async () => {
      const { search } = await render({ params: { slug: 'acme' }, owner: () => of(acme) })

      expect(search).toHaveBeenCalledWith(
        expect.objectContaining({ owner: { kind: 'organization', slug: 'acme' } }),
      )
    })
  })

  describe('counts', () => {
    it('uses the singular and drops the zero parts', async () => {
      const { el } = await render({
        owner: () => of(ownerSummary({ repository_count: 1, package_count: 0, image_count: 1 })),
      })

      expect(el.querySelector('.owner-page__counts')!.textContent).toBe('1 dépôt · 1 image')
    })

    it('shows only repositories when there are no packages nor images', async () => {
      const { el } = await render({
        owner: () => of(ownerSummary({ package_count: 0, image_count: 0 })),
      })

      expect(el.querySelector('.owner-page__counts')!.textContent).toBe('2 dépôts')
    })
  })

  it('follows the route when the owner changes', async () => {
    const { fixture, owner, params } = await render({ params: { username: 'alice' } })

    params.next(convertToParamMap({ username: 'bob' }))
    await settle(fixture)

    expect(owner).toHaveBeenLastCalledWith('personal', 'bob')
  })

  it('shows a spinner while loading, and no search yet', async () => {
    const { el, search } = await render({ owner: () => NEVER, neverSettles: true })

    expect(el.querySelector('gbt-spinner')).not.toBeNull()
    expect(el.querySelector('app-catalog-search')).toBeNull()
    expect(search).not.toHaveBeenCalled()
  })

  describe('nothing public', () => {
    it('shows the not-found state without saying whether the account exists', async () => {
      const { el, search } = await render({ owner: () => of(null) })

      const empty = el.querySelector('gbt-empty-state')!
      expect(empty.textContent).toContain('Propriétaire introuvable')
      expect(empty.textContent).toContain('rien de public à cette adresse')
      expect(el.querySelector('[role="alert"]')).toBeNull()
      expect(el.querySelector('h1')).toBeNull()
      expect(el.querySelector('app-catalog-search')).toBeNull()
      expect(search).not.toHaveBeenCalled()
    })

    it('links back to the explorer', async () => {
      const { el } = await render({ owner: () => of(null) })

      expect(el.querySelector('gbt-empty-state a')!.getAttribute('href')).toBe('/explorer')
    })
  })

  describe('failures', () => {
    it('reports a server error, not a missing owner', async () => {
      const { el } = await render({
        owner: () => throwError(() => new HttpErrorResponse({ status: 500 })),
      })

      expect(el.querySelector('[role="alert"]')!.textContent).toContain('Échec du chargement')
      expect(el.querySelector('gbt-empty-state')).toBeNull()
    })

    it('tells the visitor to slow down on a 429', async () => {
      const { el } = await render({
        owner: () => throwError(() => new HttpErrorResponse({ status: 429 })),
      })

      expect(el.querySelector('[role="alert"]')!.textContent).toContain('Trop de requêtes')
    })

    it('says the catalog is busy on a 503', async () => {
      const { el } = await render({
        owner: () => throwError(() => new HttpErrorResponse({ status: 503 })),
      })

      expect(el.querySelector('[role="alert"]')!.textContent).toContain(
        'Le catalogue est momentanément occupé',
      )
    })

    it('retries on demand', async () => {
      let attempts = 0
      const { fixture, el, owner } = await render({
        owner: () =>
          ++attempts === 1
            ? throwError(() => new HttpErrorResponse({ status: 500 }))
            : of(ownerSummary()),
      })

      button(el, 'Réessayer').click()
      await settle(fixture)

      expect(owner).toHaveBeenCalledTimes(2)
      expect(el.querySelector('[role="alert"]')).toBeNull()
      expect(el.querySelector('h1')!.textContent).toBe('admin')
    })
  })

  describe('page title', () => {
    it('is the display name once loaded', async () => {
      await render({ owner: () => of(ownerSummary({ display_name: 'Alice Martin' })) })

      expect(TestBed.inject(PageTitleService).title()).toBe('Alice Martin')
    })

    it('is generic when there is nothing to show', async () => {
      await render({ owner: () => of(null) })

      expect(TestBed.inject(PageTitleService).title()).toBe('Propriétaire')
    })
  })

  it('renders inside the public layout', async () => {
    const { el } = await render()

    expect(el.querySelector('app-public-layout header')).not.toBeNull()
  })
})
