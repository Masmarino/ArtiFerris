import { Component } from '@angular/core'
import { TestBed } from '@angular/core/testing'
import { Router, RouterLink, provideRouter } from '@angular/router'
import { of } from 'rxjs'
import { By } from '@angular/platform-browser'
import { AuthService } from '../../auth/application/auth.service'
import { CatalogService } from '../catalog/application/catalog.service'
import { catalogSuggestion } from '../catalog/testing/catalog.fixtures'
import { NO_SUGGESTIONS } from '../catalog/testing/no-suggestions'
import { PublicLayout } from './public-layout'

@Component({
  standalone: true,
  imports: [PublicLayout],
  template: '<app-public-layout><p>contenu</p></app-public-layout>',
})
class TestHost {}

@Component({ standalone: true, template: '' })
class AnyPage {}

function render(isAuthenticated = false) {
  TestBed.configureTestingModule({
    providers: [
      provideRouter([{ path: '**', component: AnyPage }]),
      NO_SUGGESTIONS,
      { provide: AuthService, useValue: { isAuthenticated: () => isAuthenticated } },
    ],
  })
  const fixture = TestBed.createComponent(PublicLayout)
  fixture.detectChanges()
  return { fixture }
}

describe('PublicLayout', () => {
  it('shows the light logo drawn for the graphite bar', () => {
    const { fixture } = render()

    const logo: HTMLImageElement = fixture.nativeElement.querySelector('img')
    expect(logo.getAttribute('src')).toBe('/Logo_horizontal.png')
  })

  it('links an anonymous visitor to /login, with the primary button', () => {
    const { fixture } = render(false)

    const link = fixture.debugElement.query(By.css('.public-layout__account'))
    expect(link.injector.get(RouterLink).href).toBe('/login')
    expect(link.nativeElement.textContent).toContain('Se connecter')
    expect(link.nativeElement.classList).toContain('gbt-button--primary')
  })

  it('comes back to the page after signing in', async () => {
    const { fixture } = render(false)
    await TestBed.inject(Router).navigateByUrl('/npm/left-pad')
    fixture.detectChanges()

    const link = fixture.debugElement.query(By.css('.public-layout__account'))
    expect(link.injector.get(RouterLink).href).toBe('/login?returnUrl=%2Fnpm%2Fleft-pad')
  })

  it('links to the documentation and to the project on GitHub, in a new tab', () => {
    const { fixture } = render()

    const [github, docs] = fixture.debugElement.queryAll(By.css('.public-layout__link'))
    expect(github.nativeElement.getAttribute('href')).toBe(
      'https://github.com/Masmarino/ArtiFerris',
    )
    expect(github.nativeElement.getAttribute('target')).toBe('_blank')
    expect(github.nativeElement.getAttribute('aria-label')).toBe(
      'ArtiFerris sur GitHub (nouvel onglet)',
    )
    expect(docs.injector.get(RouterLink).href).toBe('/docs')
    expect(docs.nativeElement.textContent).toContain('Documentation')
  })

  it('links a signed-in visitor to their dashboard instead', () => {
    const { fixture } = render(true)

    const link = fixture.debugElement.query(By.css('.public-layout__account'))
    expect(link.injector.get(RouterLink).href).toBe('/repositories')
    expect(link.nativeElement.textContent).toContain('Mes dépôts')
  })

  it('links the logo to the explorer', () => {
    const { fixture } = render()

    const link = fixture.debugElement.query(By.css('.public-layout__home'))
    expect(link.injector.get(RouterLink).href).toBe('/explorer')
    expect(link.nativeElement.querySelector('img')).not.toBeNull()
  })

  describe('header search', () => {
    function input(fixture: ReturnType<typeof render>['fixture']): HTMLInputElement {
      return fixture.nativeElement.querySelector('input[role="combobox"]')
    }

    function search(fixture: ReturnType<typeof render>['fixture'], text: string) {
      const field = input(fixture)
      field.value = text
      field.dispatchEvent(new Event('input'))
      field.dispatchEvent(
        new KeyboardEvent('keydown', { key: 'Enter', cancelable: true, bubbles: true }),
      )
    }

    function renderAt(url: string) {
      const { fixture } = render()
      const router = TestBed.inject(Router)
      vi.spyOn(router, 'url', 'get').mockReturnValue(url)
      const navigate = vi.spyOn(router, 'navigate').mockResolvedValue(true)
      return { fixture, navigate }
    }

    it('is a quick-search landmark holding a combobox with an accessible label', () => {
      const { fixture } = render()

      expect(
        fixture.nativeElement.querySelector('[role="search"][aria-label="Recherche rapide"]'),
      ).not.toBeNull()
      const field = input(fixture)
      const label: HTMLLabelElement = fixture.nativeElement.querySelector(
        `label[for="${field.id}"]`,
      )
      expect(label.textContent).toContain('Rechercher un paquet')
      expect(label.classList).toContain('gbt-input__label--hidden')
      expect(field.getAttribute('maxlength')).toBe('100')
    })

    it('navigates to the explorer with the text', () => {
      const { fixture, navigate } = renderAt('/@alice/my-lib')

      search(fixture, ' demo ')

      expect(navigate).toHaveBeenCalledWith(['/', 'explorer'], { queryParams: { q: 'demo' } })
    })

    it('stays on the current catalog page instead of going to the explorer', () => {
      const { fixture, navigate } = renderAt('/artiferris-docker?q=old&page=2')

      search(fixture, 'web')

      expect(navigate).toHaveBeenCalledWith(['/', 'artiferris-docker'], {
        queryParams: { q: 'web' },
      })
    })

    it('goes to the explorer without a q for an empty search', () => {
      const { fixture, navigate } = renderAt('/explorer')

      search(fixture, '   ')

      expect(navigate).toHaveBeenCalledWith(['/', 'explorer'], { queryParams: { q: null } })
    })

    it('suggests packages that open their own page', () => {
      vi.useFakeTimers()
      TestBed.configureTestingModule({
        providers: [
          provideRouter([]),
          { provide: AuthService, useValue: { isAuthenticated: () => false } },
          {
            provide: CatalogService,
            useValue: { suggest: () => of([catalogSuggestion({ name: 'left-pad' })]) },
          },
        ],
      })
      const navigate = vi.spyOn(TestBed.inject(Router), 'navigate').mockResolvedValue(true)
      const fixture = TestBed.createComponent(PublicLayout)
      fixture.detectChanges()
      const field = input(fixture)

      field.value = 'left'
      field.dispatchEvent(new Event('input'))
      vi.advanceTimersByTime(200)
      fixture.detectChanges()
      field.dispatchEvent(
        new KeyboardEvent('keydown', { key: 'ArrowDown', cancelable: true, bubbles: true }),
      )
      field.dispatchEvent(
        new KeyboardEvent('keydown', { key: 'Enter', cancelable: true, bubbles: true }),
      )

      expect(navigate).toHaveBeenCalledExactlyOnceWith([
        '/@admin',
        'test-npm',
        'packages',
        'npm',
        'left-pad',
      ])
      vi.useRealTimers()
    })
  })

  it('projects content', () => {
    TestBed.configureTestingModule({
      providers: [
        provideRouter([]),
        NO_SUGGESTIONS,
        { provide: AuthService, useValue: { isAuthenticated: () => false } },
      ],
    })
    const fixture = TestBed.createComponent(TestHost)
    fixture.detectChanges()

    expect(fixture.nativeElement.textContent).toContain('contenu')
  })
})
