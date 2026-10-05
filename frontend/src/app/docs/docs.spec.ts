import { Component } from '@angular/core'
import { TestBed } from '@angular/core/testing'
import { provideHttpClient } from '@angular/common/http'
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing'
import { provideRouter } from '@angular/router'
import { RouterTestingHarness } from '@angular/router/testing'
import {
  DEFAULT_DOCS_LABELS,
  DOCS_CONFIG,
  DOCS_LABELS,
  DOCS_TITLE,
  docsRoutes,
} from '@masmarino/gabarit/docs'
import { AuthService } from '../auth/application/auth.service'
import { SUPPORTED_LANGUAGES } from '../shared/i18n/languages'
import { setActiveLanguage } from '../shared/i18n/translator'
import { NO_SUGGESTIONS } from '../public/catalog/testing/no-suggestions'
import { PageTitleService } from '../shell/page-title.service'
import { docsLabelsFor } from './docs-labels'
import { provideArtiferrisDocs } from './docs-providers'
import { PublicDocsPage } from './public-docs-page'
import { ShellDocsPage } from './shell-docs-page'

const INDEX = {
  sections: [
    {
      slug: 'demarrer',
      title: 'Démarrer',
      pages: [
        { slug: 'presentation', title: 'Présentation', description: 'Ce que fait ArtiFerris.' },
      ],
    },
  ],
}

@Component({ standalone: true, providers: [provideArtiferrisDocs()], template: '' })
class ProvidersHost {}

function providedBy<T>(token: { toString(): string } & object): T {
  const fixture = TestBed.createComponent(ProvidersHost)
  return fixture.debugElement.injector.get(token as never) as T
}

describe('docsLabelsFor', () => {
  it.each(SUPPORTED_LANGUAGES.filter((language) => language !== 'en'))(
    'translates every string of the reader in %s',
    (language) => {
      const labels = docsLabelsFor(language)

      expect(Object.keys(labels).sort()).toEqual(Object.keys(DEFAULT_DOCS_LABELS).sort())
    },
  )

  it("keeps Gabarit's English strings in English", () => {
    expect(docsLabelsFor('en')).toEqual({})
  })
})

describe('provideArtiferrisDocs', () => {
  afterEach(() => setActiveLanguage('fr'))

  it('reads the pages under /docs and turns « Note » and « Attention » into callouts', () => {
    const config = providedBy<{ root: string; callouts: Record<string, string> }>(DOCS_CONFIG)

    expect(config.root).toBe('/docs')
    expect(config.callouts).toEqual({ note: 'note', attention: 'warning' })
  })

  it('speaks the interface language', () => {
    setActiveLanguage('de')

    expect(providedBy<{ previous: string }>(DOCS_LABELS).previous).toBe('Zurück')
  })

  it('puts the page title in the header and the browser tab', () => {
    providedBy<(title: string) => void>(DOCS_TITLE)('Configuration')

    expect(TestBed.inject(PageTitleService).title()).toBe('Configuration')
  })
})

describe('documentation pages', () => {
  async function open(page: typeof PublicDocsPage | typeof ShellDocsPage) {
    TestBed.configureTestingModule({
      providers: [
        provideHttpClient(),
        provideHttpClientTesting(),
        provideRouter([{ path: 'docs', children: docsRoutes(() => Promise.resolve(page)) }]),
        NO_SUGGESTIONS,
        { provide: AuthService, useValue: { isAuthenticated: () => false } },
      ],
    })
    const harness = await RouterTestingHarness.create()
    await harness.navigateByUrl('/docs/demarrer/presentation')
    const http = TestBed.inject(HttpTestingController)
    http.expectOne('/docs/index.json').flush(INDEX)
    http.expectOne('/docs/demarrer/presentation.md').flush('# Présentation\n\nUn registre.')
    await harness.fixture.whenStable()
    harness.fixture.detectChanges()
    return harness.fixture.nativeElement as HTMLElement
  }

  it('shows the reader under the public bar, over the whole width, to a visitor', async () => {
    const element = await open(PublicDocsPage)

    const content = element.querySelector('app-public-layout .public-layout__content--full')
    expect(content?.querySelector('gbt-docs-page h1')?.textContent).toContain('Présentation')
  })

  it('shows the reader alone inside the shell to a signed-in user', async () => {
    const element = await open(ShellDocsPage)

    expect(element.querySelector('app-public-layout')).toBeNull()
    expect(element.querySelector('gbt-docs-page h1')?.textContent).toContain('Présentation')
    expect(TestBed.inject(PageTitleService).title()).toBe('Présentation')
  })
})
