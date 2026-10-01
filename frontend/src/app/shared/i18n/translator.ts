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

export const activeLanguage = language.asReadonly()

export const activeLocale = computed(() => LANGUAGE_LOCALES[language()])

export function setActiveLanguage(next: Language): void {
  language.set(next)
}

export function registerTranslator(translate: TranslateFn): void {
  current = translate
}

/**
 * Looks a key up in the active language, for code outside templates and injection contexts. A
 * `computed` calling it follows language changes.
 */
export const t: TranslateFn = (key, params) => {
  language()
  if (!current) {
    throw new Error(`Translator not registered: cannot translate "${key}" yet`)
  }
  return current(key, params)
}

export function provideTranslator(): EnvironmentProviders {
  return makeEnvironmentProviders([
    provideEnvironmentInitializer(() => {
      const transloco = inject(TranslocoService)
      registerTranslator((key, params) => transloco.translate(key, params))
    }),
  ])
}
