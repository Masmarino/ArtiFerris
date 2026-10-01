import {
  EnvironmentProviders,
  Injectable,
  Provider,
  inject,
  provideEnvironmentInitializer,
} from '@angular/core'
import { TestBed } from '@angular/core/testing'
import {
  Translation,
  TranslocoLoader,
  TranslocoService,
  provideTransloco,
} from '@jsverse/transloco'
import { Observable, of } from 'rxjs'
import fr from '../public/i18n/fr.json'
import {
  provideTranslator,
  registerTranslator,
  setActiveLanguage,
} from './app/shared/i18n/translator'

/** Serves the real French dictionary synchronously, so specs render the app's text. */
@Injectable()
class InlineTranslocoLoader implements TranslocoLoader {
  getTranslation(): Observable<Translation> {
    return of(fr as Translation)
  }
}

/**
 * Just enough of Transloco's lookup + `{{ param }}` interpolation for specs that never build a
 * TestBed.
 */
function lookup(key: string, params: Record<string, unknown> = {}): string {
  const value = key
    .split('.')
    .reduce<unknown>((node, part) => (node as Record<string, unknown> | undefined)?.[part], fr)
  if (typeof value !== 'string') {
    return key
  }
  return value.replace(/\{\{\s*(\w+)\s*\}\}/g, (_, name: string) => String(params[name] ?? ''))
}

// Every spec gets Transloco with the French dictionary; a spec can still provide its own.
const i18nProviders: (Provider | EnvironmentProviders)[] = [
  provideTransloco({
    config: { availableLangs: ['fr'], defaultLang: 'fr', prodMode: false },
    loader: InlineTranslocoLoader,
  }),
  provideTranslator(),
  provideEnvironmentInitializer(() => {
    inject(TranslocoService).load('fr').subscribe()
  }),
]

// resetTestingModule() drops these providers, so they are re-applied after each reset.
const resetTestingModule = TestBed.resetTestingModule.bind(TestBed)
TestBed.resetTestingModule = () => {
  const testBed = resetTestingModule()
  testBed.configureTestingModule({ providers: i18nProviders })
  return testBed
}

beforeEach(() => {
  // Specs with no TestBed still get French text; those with one use the real service.
  registerTranslator(lookup)
  setActiveLanguage('fr')
  TestBed.configureTestingModule({ providers: i18nProviders })
})
