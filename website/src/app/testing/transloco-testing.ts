import { Component, EnvironmentProviders, Provider, Type } from '@angular/core'
import { TestBed } from '@angular/core/testing'
import { Routes, UrlSerializer, provideRouter } from '@angular/router'
import { TranslocoService, provideTransloco } from '@jsverse/transloco'
import { firstValueFrom } from 'rxjs'
import { AVAILABLE_LANGS, StaticTranslocoLoader } from '../i18n/static-transloco.loader'
import { DEFAULT_LANG, Lang } from '../i18n/languages'
import { TrailingSlashUrlSerializer } from '../shared/trailing-slash-url-serializer'

@Component({ template: '' })
class Blank {}

/** One catch-all route, for components that only need the router to build and match links. */
export const BLANK_ROUTES: Routes = [{ path: '**', component: Blank }]

export function provideTestTransloco(): (Provider | EnvironmentProviders)[] {
  return provideTransloco({
    config: { availableLangs: AVAILABLE_LANGS, defaultLang: DEFAULT_LANG, prodMode: true },
    loader: StaticTranslocoLoader,
  })
}

/** Loads a dictionary and makes it the active one, as the route resolver does before a page is created. */
export async function activateLang(service: TranslocoService, lang: Lang): Promise<void> {
  await firstValueFrom(service.load(lang))
  service.setActiveLang(lang)
}

/** The router as the app configures it: same URL serializer, so hrefs end with a slash. */
export function provideTestRouter(routes: Routes): (Provider | EnvironmentProviders)[] {
  return [provideRouter(routes), { provide: UrlSerializer, useClass: TrailingSlashUrlSerializer }]
}

/** Configures TestBed for components that render translated text, with the router when `routes` is given. */
export async function setUpTestApp(options: {
  imports: Type<unknown>[]
  lang: Lang
  routes?: Routes
}): Promise<void> {
  await TestBed.configureTestingModule({
    imports: options.imports,
    providers: [
      provideTestTransloco(),
      ...(options.routes ? provideTestRouter(options.routes) : []),
    ],
  }).compileComponents()
  await activateLang(TestBed.inject(TranslocoService), options.lang)
}
