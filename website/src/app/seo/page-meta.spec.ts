import { LANGS } from '../i18n/languages'
import { alternateLinks, applyNotFoundSeo, applyPageSeo, buildPageSeo, pageUrl } from './page-meta'

function seoFor(lang: (typeof LANGS)[number], page: 'home' | 'features' | 'roadmap') {
  return buildPageSeo({
    lang,
    page,
    title: `Title ${lang}`,
    description: `Description ${lang}`,
    imageAlt: 'alt',
  })
}

describe('page-meta', () => {
  it('builds absolute URLs that end with a slash', () => {
    expect(pageUrl('en', 'home')).toBe('https://www.artiferris.pro/en/')
    expect(pageUrl('fr', 'features')).toBe('https://www.artiferris.pro/fr/features/')
    expect(pageUrl('de', 'roadmap')).toBe('https://www.artiferris.pro/de/roadmap/')
  })

  it('lists one alternate per language plus x-default pointing at English', () => {
    const alternates = alternateLinks('features')
    expect(alternates.map((link) => link.hreflang)).toEqual([
      'en',
      'fr',
      'it',
      'es',
      'de',
      'x-default',
    ])
    expect(alternates.find((link) => link.hreflang === 'x-default')?.href).toBe(
      'https://www.artiferris.pro/en/features/',
    )
    expect(alternates.find((link) => link.hreflang === 'it')?.href).toBe(
      'https://www.artiferris.pro/it/features/',
    )
  })

  it('writes the head of a page: title, description, canonical, hreflang and Open Graph', () => {
    applyPageSeo(document, seoFor('fr', 'roadmap'))

    expect(document.documentElement.lang).toBe('fr')
    expect(document.title).toBe('Title fr')
    expect(document.querySelector('meta[name="description"]')?.getAttribute('content')).toBe(
      'Description fr',
    )
    expect(document.querySelector('link[rel="canonical"]')?.getAttribute('href')).toBe(
      'https://www.artiferris.pro/fr/roadmap/',
    )
    const hreflangs = Array.from(document.querySelectorAll('link[rel="alternate"][hreflang]')).map(
      (link) => [link.getAttribute('hreflang'), link.getAttribute('href')],
    )
    expect(hreflangs).toHaveLength(6)
    expect(hreflangs).toContainEqual(['es', 'https://www.artiferris.pro/es/roadmap/'])
    expect(document.querySelector('meta[property="og:url"]')?.getAttribute('content')).toBe(
      'https://www.artiferris.pro/fr/roadmap/',
    )
    expect(document.querySelector('meta[property="og:locale"]')?.getAttribute('content')).toBe(
      'fr_FR',
    )
    const alternateLocales = Array.from(
      document.querySelectorAll('meta[property="og:locale:alternate"]'),
    ).map((meta) => meta.getAttribute('content'))
    expect(alternateLocales).toEqual(['en_US', 'it_IT', 'es_ES', 'de_DE'])
    expect(document.querySelector('meta[property="og:image"]')?.getAttribute('content')).toBe(
      'https://www.artiferris.pro/images/og-fr.png',
    )
    expect(document.querySelector('meta[name="twitter:card"]')?.getAttribute('content')).toBe(
      'summary_large_image',
    )
  })

  it('replaces the previous page instead of piling tags up', () => {
    applyPageSeo(document, seoFor('en', 'home'))
    applyPageSeo(document, seoFor('de', 'features'))

    expect(document.querySelectorAll('link[rel="canonical"]')).toHaveLength(1)
    expect(document.querySelectorAll('link[rel="alternate"][hreflang]')).toHaveLength(6)
    expect(document.querySelectorAll('meta[property="og:title"]')).toHaveLength(1)
    expect(document.querySelector('link[rel="canonical"]')?.getAttribute('href')).toBe(
      'https://www.artiferris.pro/de/features/',
    )
  })

  it('adds SoftwareApplication structured data on request only, with the license of the repository', () => {
    applyPageSeo(document, seoFor('en', 'home'), { jsonLd: true })
    const script = document.getElementById('artiferris-jsonld')
    const data = JSON.parse(script?.textContent ?? '{}')
    expect(data['@type']).toContain('SoftwareApplication')
    expect(data.name).toBe('ArtiFerris')
    expect(data.url).toBe('https://www.artiferris.pro/en/')
    expect(data.codeRepository).toBe('https://github.com/Masmarino/ArtiFerris')
    expect(data.license).toBe('https://www.apache.org/licenses/LICENSE-2.0')

    applyPageSeo(document, seoFor('en', 'features'))
    expect(document.getElementById('artiferris-jsonld')).toBeNull()
  })

  it('keeps the 404 page out of the index and without canonical or alternates', () => {
    applyPageSeo(document, seoFor('en', 'home'))
    applyNotFoundSeo(document, 'Page not found', 'Missing')

    expect(document.querySelector('meta[name="robots"]')?.getAttribute('content')).toBe('noindex')
    expect(document.querySelector('link[rel="canonical"]')).toBeNull()
    expect(document.querySelectorAll('link[rel="alternate"][hreflang]')).toHaveLength(0)
    expect(document.documentElement.lang).toBe('en')
  })
})
