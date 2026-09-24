import { TestBed } from '@angular/core/testing'
import { ActivatedRoute, convertToParamMap, provideRouter } from '@angular/router'
import { of } from 'rxjs'
import { AuthService } from '../../../auth/application/auth.service'
import { PageTitleService } from '../../../shell/page-title.service'
import { CatalogService } from '../application/catalog.service'
import { CatalogSearch } from '../catalog-search/catalog-search'
import { catalogEntry, searchResult } from '../testing/catalog.fixtures'
import { CatalogPage } from './catalog-page'

async function render(data: Record<string, string>) {
  const search = vi.fn().mockReturnValue(of(searchResult([catalogEntry()])))
  TestBed.configureTestingModule({
    providers: [
      provideRouter([]),
      {
        provide: ActivatedRoute,
        useValue: { snapshot: { data }, queryParamMap: of(convertToParamMap({})) },
      },
      { provide: CatalogService, useValue: { search } },
      { provide: AuthService, useValue: { isAuthenticated: () => false } },
    ],
  })
  const fixture = TestBed.createComponent(CatalogPage)
  fixture.detectChanges()
  await fixture.whenStable()
  fixture.detectChanges()
  return { fixture, el: fixture.nativeElement as HTMLElement, search }
}

describe('CatalogPage', () => {
  it('titles the page with the catalog name and describes the npm catalog', async () => {
    const { el } = await render({ catalogName: 'artiferris-npm', format: 'npm' })

    expect(el.querySelector('h1')!.textContent).toBe('artiferris-npm')
    expect(el.querySelector('.catalog-page__header p')!.textContent).toBe(
      "Tous les paquets npm publics hébergés sur cette instance. On installe toujours depuis l'URL du propriétaire.",
    )
  })

  it('describes the docker catalog', async () => {
    const { el } = await render({ catalogName: 'artiferris-docker', format: 'docker' })

    expect(el.querySelector('h1')!.textContent).toBe('artiferris-docker')
    expect(el.querySelector('.catalog-page__header p')!.textContent).toContain('images Docker')
  })

  it('locks the search to the catalog format', async () => {
    const { fixture, el, search } = await render({ catalogName: 'artiferris-npm', format: 'npm' })

    const catalogSearch = fixture.debugElement.children[0].query(
      (d) => d.name === 'app-catalog-search',
    )
    expect(catalogSearch.componentInstance).toBeInstanceOf(CatalogSearch)
    expect(catalogSearch.componentInstance.format()).toBe('npm')
    expect(search).toHaveBeenCalledWith(expect.objectContaining({ format: 'npm' }))
    expect(el.querySelector('[aria-label="Format"]')).toBeNull()
  })

  it('sets the browser page title', async () => {
    await render({ catalogName: 'artiferris-npm', format: 'npm' })

    expect(TestBed.inject(PageTitleService).title()).toBe('artiferris-npm')
  })

  it('renders inside the public layout', async () => {
    const { el } = await render({ catalogName: 'artiferris-npm', format: 'npm' })

    expect(el.querySelector('app-public-layout header')).not.toBeNull()
  })
})
