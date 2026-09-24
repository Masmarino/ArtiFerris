import { TestBed } from '@angular/core/testing'
import { By } from '@angular/platform-browser'
import { RouterLink, provideRouter } from '@angular/router'
import { CatalogEntry } from '../domain/catalog.entity'
import { catalogEntry, dockerEntry } from '../testing/catalog.fixtures'
import { CatalogResultCard } from './catalog-result-card'

function render(entry: CatalogEntry) {
  TestBed.configureTestingModule({ providers: [provideRouter([])] })
  const fixture = TestBed.createComponent(CatalogResultCard)
  fixture.componentRef.setInput('entry', entry)
  fixture.detectChanges()
  const el: HTMLElement = fixture.nativeElement
  return { fixture, el }
}

function hrefs(fixture: ReturnType<typeof render>['fixture']): string[] {
  return fixture.debugElement
    .queryAll(By.directive(RouterLink))
    .map((link) => link.injector.get(RouterLink).href ?? '')
}

function linkTargets(el: HTMLElement): [string, string | null][] {
  return Array.from(el.querySelectorAll('a')).map((a) => [
    a.textContent!.trim(),
    a.getAttribute('href'),
  ])
}

describe('CatalogResultCard', () => {
  afterEach(() => {
    vi.useRealTimers()
    vi.restoreAllMocks()
  })

  it('shows the format, the name as a link to the package page, and the version badge', () => {
    const { fixture, el } = render(catalogEntry())

    expect(el.textContent).toContain('npm')
    expect(el.querySelector('h3 a')?.textContent).toBe('hangar-demo')
    expect(hrefs(fixture)).toContain('/@admin/test-npm/packages/npm/hangar-demo')
    expect(el.textContent).toContain('1.1.0')
  })

  it('encodes a docker name containing a slash into a single path segment', () => {
    const { fixture } = render(dockerEntry())

    expect(hrefs(fixture)).toContain('/o/acme/images/packages/docker/team%2Fapi')
  })

  it('shows the description and one chip per keyword', () => {
    const { el } = render(catalogEntry())

    expect(el.querySelector('.result-card__description')?.textContent).toContain('A small demo')
    expect(
      Array.from(el.querySelectorAll('.result-card__keywords li')).map((li) => li.textContent),
    ).toEqual(['demo', 'catalog'])
  })

  it('omits the description, keywords and version badge when the entry has none', () => {
    const { el } = render(catalogEntry({ description: null, keywords: [], latest: null }))

    expect(el.querySelector('.result-card__description')).toBeNull()
    expect(el.querySelector('.result-card__keywords')).toBeNull()
    expect(el.querySelector('gbt-badge')).toBeNull()
  })

  it('credits a personal owner, linking the owner to their profile and the repository to its page', () => {
    const { el } = render(catalogEntry())

    expect(el.querySelector('.result-card__meta')?.textContent).toContain('par admin / test-npm')
    expect(linkTargets(el)).toEqual(
      expect.arrayContaining([
        ['admin', '/@admin'],
        ['test-npm', '/@admin/test-npm'],
      ]),
    )
  })

  it('credits an organization by its display name, with links under /o', () => {
    const { el } = render(dockerEntry())

    expect(el.querySelector('.result-card__meta')?.textContent).toContain('par Acme Corp / images')
    expect(linkTargets(el)).toEqual(
      expect.arrayContaining([
        ['Acme Corp', '/o/acme'],
        ['images', '/o/acme/images'],
      ]),
    )
  })

  it('shows a relative update date, with the exact one as a tooltip', () => {
    vi.useFakeTimers({ toFake: ['Date'] })
    vi.setSystemTime(new Date('2026-09-27T11:20:00Z'))
    const { el } = render(catalogEntry())

    const time = el.querySelector('time')!
    expect(time.textContent).toContain('il y a 3 jours')
    expect(time.getAttribute('datetime')).toBe('2026-09-24T11:20:00Z')
    expect(time.getAttribute('title')).toBeTruthy()
  })

  it('shows the weekly downloads next to the update date, with thousands separated', () => {
    const { el } = render(catalogEntry({ downloads_7d: 1234 }))

    expect(el.querySelector('.result-card__downloads')?.textContent).toBe(
      '1\u202f234 téléchargements cette semaine',
    )
  })

  it('keeps one download singular', () => {
    const { el } = render(catalogEntry({ downloads_7d: 1 }))

    expect(el.querySelector('.result-card__downloads')?.textContent).toBe(
      '1 téléchargement cette semaine',
    )
  })

  it('shows nothing about downloads when there were none', () => {
    const { el } = render(catalogEntry({ downloads_7d: 0 }))

    expect(el.querySelector('.result-card__downloads')).toBeNull()
    expect(el.textContent).not.toContain('téléchargement')
  })

  it('shows the npm install command from the owner registry URL', () => {
    const { el } = render(catalogEntry())

    expect(el.querySelector('pre')?.textContent).toBe(
      'npm install hangar-demo --registry http://localhost:4200/npm/u/admin/test-npm/',
    )
  })

  it('shows the docker pull command from the image reference and latest tag', () => {
    const { el } = render(dockerEntry())

    expect(el.querySelector('pre')?.textContent).toBe(
      'docker pull localhost:4200/o/acme/images/team/api:v2.0.1',
    )
  })

  describe('copy button', () => {
    function copyButton(el: HTMLElement): HTMLButtonElement {
      return el.querySelector('gbt-button button')!
    }

    it('writes the install command to the clipboard and confirms it', async () => {
      const writeText = vi.fn().mockResolvedValue(undefined)
      vi.stubGlobal('navigator', { clipboard: { writeText } })
      const { fixture, el } = render(catalogEntry())

      copyButton(el).click()
      await fixture.whenStable()
      fixture.detectChanges()

      expect(writeText).toHaveBeenCalledWith(
        'npm install hangar-demo --registry http://localhost:4200/npm/u/admin/test-npm/',
      )
      expect(el.querySelector('[aria-live="polite"]')?.textContent).toBe('Commande copiée')
      vi.unstubAllGlobals()
    })

    it('reports a failure when the clipboard is unavailable', async () => {
      vi.stubGlobal('navigator', {
        clipboard: { writeText: vi.fn().mockRejectedValue(new Error('denied')) },
      })
      const { fixture, el } = render(catalogEntry())

      copyButton(el).click()
      await fixture.whenStable()
      fixture.detectChanges()

      expect(el.querySelector('[aria-live="polite"]')?.textContent).toContain('Copie impossible')
      vi.unstubAllGlobals()
    })

    it('goes back to idle after a couple of seconds', async () => {
      vi.stubGlobal('navigator', { clipboard: { writeText: vi.fn().mockResolvedValue(undefined) } })
      vi.useFakeTimers()
      const { fixture, el } = render(catalogEntry())

      copyButton(el).click()
      await vi.advanceTimersByTimeAsync(0)
      fixture.detectChanges()
      expect(el.querySelector('[aria-live="polite"]')?.textContent).toBe('Commande copiée')

      await vi.advanceTimersByTimeAsync(2000)
      fixture.detectChanges()

      expect(el.querySelector('[aria-live="polite"]')?.textContent).toBe('')
      vi.unstubAllGlobals()
    })

    it('has an accessible name mentioning the package', () => {
      const { el } = render(catalogEntry())

      expect(copyButton(el).getAttribute('aria-label')).toBe(
        "Copier la commande d'installation de hangar-demo",
      )
    })
  })
})
