import { Injectable, inject, provideAppInitializer } from '@angular/core'
import {
  Translation,
  TranslocoLoader,
  TranslocoService,
  provideTransloco,
} from '@jsverse/transloco'
import { Observable, firstValueFrom, of } from 'rxjs'
import { provideTranslator } from '../src/app/shared/i18n/translator'
import fr from '../public/i18n/fr.json'

/** Storybook has no server for the dictionary, so it is bundled. */
@Injectable()
class BundledTranslocoLoader implements TranslocoLoader {
  getTranslation(): Observable<Translation> {
    return of(fr as Translation)
  }
}

export function provideStorybookTransloco() {
  return [
    provideTransloco({
      config: { availableLangs: ['fr'], defaultLang: 'fr', prodMode: false },
      loader: BundledTranslocoLoader,
    }),
    provideTranslator(),
    provideAppInitializer(() => {
      const transloco = inject(TranslocoService)
      return firstValueFrom(transloco.load('fr'))
    }),
  ]
}
