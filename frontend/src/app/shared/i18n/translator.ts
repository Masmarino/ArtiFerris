import {
  EnvironmentProviders,
  inject,
  makeEnvironmentProviders,
  provideEnvironmentInitializer,
} from '@angular/core'
import { TranslocoService } from '@jsverse/transloco'
import { TranslateFn } from './translate-fn'

let current: TranslateFn | null = null

/** Points `t()` at the running application's translations (or at a test double). */
export function registerTranslator(translate: TranslateFn): void {
  current = translate
}

/**
 * Looks a key up in the active language. Meant for code that builds user-facing text outside a
 * template and outside an injection context — validators, formatters, error mappers. A
 * component that already has an injector should still prefer `TranslocoService`.
 */
export const t: TranslateFn = (key, params) => {
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
