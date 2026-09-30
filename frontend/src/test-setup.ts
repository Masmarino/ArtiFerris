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
import { provideTranslator, registerTranslator } from './app/shared/i18n/translator'

/** Serves the real French dictionary synchronously, so specs render the same text as the app. */
@Injectable()
class InlineTranslocoLoader implements TranslocoLoader {
  getTranslation(): Observable<Translation> {
    return of(fr as Translation)
  }
}

/** Just enough of Transloco's lookup + `{{ param }}` interpolation for specs that never build a TestBed. */
function lookup(key: string, params: Record<string, unknown> = {}): string {
  const value = key
    .split('.')
    .reduce<unknown>((node, part) => (node as Record<string, unknown> | undefined)?.[part], fr)
  if (typeof value !== 'string') {
    return key
  }
  return value.replace(/\{\{\s*(\w+)\s*\}\}/g, (_, name: string) => String(params[name] ?? ''))
}

// Every spec gets Transloco with the French dictionary already loaded; a spec that needs a
// different configuration can still provide its own, which takes precedence.
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

// A spec that calls resetTestingModule() mid-test drops every provider configured so far, so
// they are re-applied right after each reset.
const resetTestingModule = TestBed.resetTestingModule.bind(TestBed)
TestBed.resetTestingModule = () => {
  const testBed = resetTestingModule()
  testBed.configureTestingModule({ providers: i18nProviders })
  return testBed
}

beforeEach(() => {
  // Specs that call a translating helper directly, without any TestBed, still get French text;
  // those that do build a TestBed replace this with the real Transloco service.
  registerTranslator(lookup)
  TestBed.configureTestingModule({ providers: i18nProviders })
})
