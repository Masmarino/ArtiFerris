import {
  EnvironmentProviders,
  computed,
  inject,
  makeEnvironmentProviders,
  provideEnvironmentInitializer,
  signal,
} from '@angular/core'
import { TranslocoService } from '@jsverse/transloco'
import { FALLBACK_LANGUAGE, LANGUAGE_LOCALES, Language } from './languages'
import { TranslateFn } from './translate-fn'

let current: TranslateFn | null = null

const language = signal<Language>(FALLBACK_LANGUAGE)

/** The language the interface is displayed in. Read it in a `computed` to follow a language change. */
export const activeLanguage = language.asReadonly()

/** The BCP 47 locale (`fr-FR`…) of the active language, for dates and numbers. */
export const activeLocale = computed(() => LANGUAGE_LOCALES[language()])

/** Records which language is displayed; call it once that language's dictionary is loaded. */
export function setActiveLanguage(next: Language): void {
  language.set(next)
}

/** Points `t()` at the running application's translations (or at a test double). */
export function registerTranslator(translate: TranslateFn): void {
  current = translate
}

/**
 * Looks a key up in the active language. Meant for code that builds user-facing text outside a
 * template and outside an injection context — validators, formatters, error mappers. A
 * `computed` that calls it is re-evaluated when the language changes (it reads `activeLanguage`).
 */
export const t: TranslateFn = (key, params) => {
  language()
  if (!current) {
    throw new Error(`Translator not registered: cannot translate "${key}" yet`)
  }
  return current(key, params)
}

/** Wires `t()` to the Transloco instance of the current injector. */
export function provideTranslator(): EnvironmentProviders {
  return makeEnvironmentProviders([
    provideEnvironmentInitializer(() => {
      const transloco = inject(TranslocoService)
      registerTranslator((key, params) => transloco.translate(key, params))
    }),
  ])
}
