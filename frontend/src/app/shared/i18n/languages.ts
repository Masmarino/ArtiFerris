export const LANGUAGE_LOCALES = {
  en: 'en-US',
  fr: 'fr-FR',
  es: 'es-ES',
  it: 'it-IT',
  de: 'de-DE',
} as const

export type Language = keyof typeof LANGUAGE_LOCALES

export const SUPPORTED_LANGUAGES = Object.keys(LANGUAGE_LOCALES) as Language[]

export const LANGUAGE_NAMES: Record<Language, string> = {
  en: 'English',
  fr: 'Français',
  es: 'Español',
  it: 'Italiano',
  de: 'Deutsch',
}

export const FALLBACK_LANGUAGE: Language = 'en'

export function isSupported(code: string): code is Language {
  return Object.hasOwn(LANGUAGE_LOCALES, code)
}

/**
 * The first of `preferred` (a browser's `navigator.languages`, most wanted first) that is
 * translated, comparing only the language part: `fr-CA` picks `fr`. English otherwise.
 */
export function pickLanguage(preferred: readonly string[]): Language {
  for (const tag of preferred) {
    const code = tag.trim().split(/[-_]/)[0]?.toLowerCase() ?? ''
    if (isSupported(code)) {
      return code
    }
  }
  return FALLBACK_LANGUAGE
}

export function detectBrowserLanguage(): Language {
  return pickLanguage(globalThis.navigator?.languages ?? [])
}
