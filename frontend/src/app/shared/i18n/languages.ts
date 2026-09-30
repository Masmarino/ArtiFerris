/** The languages the interface is translated into, each with the BCP 47 locale it formats dates and numbers with. */
export const LANGUAGE_LOCALES = {
  en: 'en-US',
  fr: 'fr-FR',
  es: 'es-ES',
  it: 'it-IT',
  de: 'de-DE',
} as const

export type Language = keyof typeof LANGUAGE_LOCALES

export const SUPPORTED_LANGUAGES = Object.keys(LANGUAGE_LOCALES) as Language[]

/** Used when none of the browser's languages is translated. */
export const FALLBACK_LANGUAGE: Language = 'en'

function isSupported(code: string): code is Language {
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

/** The language for the browser this code runs in. */
export function detectBrowserLanguage(): Language {
  return pickLanguage(globalThis.navigator?.languages ?? [])
}
