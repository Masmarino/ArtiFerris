import site from '../seo/site.json'

export type Lang = 'en' | 'fr' | 'it' | 'es' | 'de'
export type PageId = 'home' | 'features' | 'registries' | 'install' | 'security' | 'roadmap'

// site.json is also read by scripts/build-sitemap.mjs, so the sitemap and the router share one list.
export const LANGS = site.langs as readonly Lang[]
export const DEFAULT_LANG = site.defaultLang as Lang
export const PAGES = site.pages as Record<PageId, string>
export const PAGE_IDS = Object.keys(PAGES) as PageId[]

// Each language is named in itself, so a visitor can find theirs on a page they cannot read.
export const LANG_NAMES: Record<Lang, string> = {
  en: 'English',
  fr: 'Français',
  it: 'Italiano',
  es: 'Español',
  de: 'Deutsch',
}

export const OG_LOCALES: Record<Lang, string> = {
  en: 'en_US',
  fr: 'fr_FR',
  it: 'it_IT',
  es: 'es_ES',
  de: 'de_DE',
}

export function isLang(value: unknown): value is Lang {
  return typeof value === 'string' && (LANGS as readonly string[]).includes(value)
}

export function pageIdFromPath(path: string): PageId {
  const segment = path.split('?')[0].split('#')[0].split('/').filter(Boolean)[1] ?? ''
  return PAGE_IDS.find((id) => PAGES[id] === segment) ?? 'home'
}
