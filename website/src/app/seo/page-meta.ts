import { effect, inject } from '@angular/core'
import { DOCUMENT } from '@angular/common'
import { TranslocoService } from '@jsverse/transloco'
import site from './site.json'
import { injectActiveLang } from '../i18n/active-lang'
import { DEFAULT_LANG, LANGS, Lang, OG_LOCALES, PAGES, PageId } from '../i18n/languages'
import { GITHUB_URL } from '../shared/links'

const SITE_NAME = 'ArtiFerris'
const SITE_URL = site.siteUrl
const OG_IMAGE = { width: 1200, height: 630 }

const JSON_LD_ID = 'artiferris-jsonld'

export interface AlternateLink {
  hreflang: string
  href: string
}

export interface PageSeo {
  lang: Lang
  title: string
  description: string
  canonical: string
  alternates: AlternateLink[]
  ogLocale: string
  ogLocaleAlternates: string[]
  image: string
  imageAlt: string
}

/** Absolute URL of a page. Directories end with a slash: that is how nginx serves the prerendered files. */
export function pageUrl(lang: Lang, page: PageId): string {
  const path = PAGES[page]
  return `${SITE_URL}/${lang}/${path ? `${path}/` : ''}`
}

/** One alternate per language, plus x-default pointing at the default language. */
export function alternateLinks(page: PageId): AlternateLink[] {
  return [
    ...LANGS.map((lang) => ({ hreflang: lang, href: pageUrl(lang, page) })),
    { hreflang: 'x-default', href: pageUrl(DEFAULT_LANG, page) },
  ]
}

export function buildPageSeo(input: {
  lang: Lang
  page: PageId
  title: string
  description: string
  imageAlt: string
}): PageSeo {
  const { lang, page } = input
  return {
    lang,
    title: input.title,
    description: input.description,
    canonical: pageUrl(lang, page),
    alternates: alternateLinks(page),
    ogLocale: OG_LOCALES[lang],
    ogLocaleAlternates: LANGS.filter((other) => other !== lang).map((other) => OG_LOCALES[other]),
    image: `${SITE_URL}/images/og-${lang}.png`,
    imageAlt: input.imageAlt,
  }
}

/** Structured data for the home page. The license is the repository's: Apache 2.0. */
function softwareApplicationJsonLd(seo: PageSeo): Record<string, unknown> {
  return {
    '@context': 'https://schema.org',
    '@type': ['SoftwareApplication', 'SoftwareSourceCode'],
    name: SITE_NAME,
    description: seo.description,
    url: seo.canonical,
    inLanguage: seo.lang,
    applicationCategory: 'DeveloperApplication',
    operatingSystem: 'Docker, Kubernetes',
    programmingLanguage: 'Rust',
    codeRepository: GITHUB_URL,
    license: 'https://www.apache.org/licenses/LICENSE-2.0',
    image: seo.image,
  }
}

function setMeta(
  doc: Document,
  attribute: 'name' | 'property',
  key: string,
  content: string,
): void {
  let element = doc.head.querySelector<HTMLMetaElement>(`meta[${attribute}="${key}"]`)
  if (!element) {
    element = doc.createElement('meta')
    element.setAttribute(attribute, key)
    doc.head.appendChild(element)
  }
  element.setAttribute('content', content)
}

function removeAll(doc: Document, selector: string): void {
  doc.head.querySelectorAll(selector).forEach((element) => element.remove())
}

function setLink(doc: Document, rel: string, href: string, hreflang?: string): void {
  const element = doc.createElement('link')
  element.setAttribute('rel', rel)
  if (hreflang) element.setAttribute('hreflang', hreflang)
  element.setAttribute('href', href)
  doc.head.appendChild(element)
}

/**
 * Writes the page head. Hydration doesn't manage <head>, so this runs again in the browser and has to replace what the
 * prerender wrote instead of adding to it.
 */
export function applyPageSeo(
  doc: Document,
  seo: PageSeo,
  options: { jsonLd?: boolean; noindex?: boolean } = {},
): void {
  doc.documentElement.lang = seo.lang
  doc.title = seo.title

  setMeta(doc, 'name', 'description', seo.description)
  setMeta(doc, 'property', 'og:type', 'website')
  setMeta(doc, 'property', 'og:site_name', SITE_NAME)
  setMeta(doc, 'property', 'og:title', seo.title)
  setMeta(doc, 'property', 'og:description', seo.description)
  setMeta(doc, 'property', 'og:url', seo.canonical)
  setMeta(doc, 'property', 'og:locale', seo.ogLocale)
  setMeta(doc, 'property', 'og:image', seo.image)
  setMeta(doc, 'property', 'og:image:width', String(OG_IMAGE.width))
  setMeta(doc, 'property', 'og:image:height', String(OG_IMAGE.height))
  setMeta(doc, 'property', 'og:image:alt', seo.imageAlt)
  setMeta(doc, 'name', 'twitter:card', 'summary_large_image')
  setMeta(doc, 'name', 'twitter:title', seo.title)
  setMeta(doc, 'name', 'twitter:description', seo.description)
  setMeta(doc, 'name', 'twitter:image', seo.image)
  setMeta(doc, 'name', 'twitter:image:alt', seo.imageAlt)

  removeAll(doc, 'meta[property="og:locale:alternate"]')
  for (const locale of seo.ogLocaleAlternates) {
    const element = doc.createElement('meta')
    element.setAttribute('property', 'og:locale:alternate')
    element.setAttribute('content', locale)
    doc.head.appendChild(element)
  }

  removeAll(doc, 'link[rel="canonical"], link[rel="alternate"][hreflang]')
  setLink(doc, 'canonical', seo.canonical)
  for (const alternate of seo.alternates) {
    setLink(doc, 'alternate', alternate.href, alternate.hreflang)
  }

  if (options.noindex) setMeta(doc, 'name', 'robots', 'noindex')
  else removeAll(doc, 'meta[name="robots"]')

  doc.getElementById(JSON_LD_ID)?.remove()
  if (options.jsonLd) {
    const script = doc.createElement('script')
    script.setAttribute('type', 'application/ld+json')
    script.id = JSON_LD_ID
    // Escape "<" so no value can close the script element early.
    script.textContent = JSON.stringify(softwareApplicationJsonLd(seo)).replace(/</g, '\\u003c')
    doc.head.appendChild(script)
  }
}

/** The 404 page is English, has no alternates and must stay out of search results. */
export function applyNotFoundSeo(doc: Document, title: string, description: string): void {
  const seo = buildPageSeo({ lang: 'en', page: 'home', title, description, imageAlt: SITE_NAME })
  applyPageSeo(doc, seo, { noindex: true })
  removeAll(doc, 'link[rel="canonical"], link[rel="alternate"][hreflang]')
  removeAll(doc, 'meta[property="og:url"]')
}

export function usePageMeta(page: PageId): void {
  const transloco = inject(TranslocoService)
  const doc = inject(DOCUMENT)
  const activeLang = injectActiveLang()

  effect(() => {
    const seo = buildPageSeo({
      lang: activeLang(),
      page,
      title: transloco.translate(`meta.${page}.title`),
      description: transloco.translate(`meta.${page}.description`),
      imageAlt: transloco.translate('meta.imageAlt'),
    })
    applyPageSeo(doc, seo, { jsonLd: page === 'home' })
  })
}

export function useNotFoundMeta(): void {
  const transloco = inject(TranslocoService)
  const doc = inject(DOCUMENT)
  effect(() => {
    applyNotFoundSeo(
      doc,
      `${transloco.translate('notFound.title')} | ${SITE_NAME}`,
      transloco.translate('notFound.description'),
    )
  })
}
