import {
  ApplicationConfig,
  inject,
  provideEnvironmentInitializer,
  provideZonelessChangeDetection,
} from '@angular/core'
import { ViewportScroller } from '@angular/common'
import { provideClientHydration, withNoIncrementalHydration } from '@angular/platform-browser'
import {
  Router,
  UrlSerializer,
  provideRouter,
  withInMemoryScrolling,
  withNavigationErrorHandler,
  withViewTransitions,
} from '@angular/router'
import { provideTransloco } from '@jsverse/transloco'
import { IconRegistry } from '@masmarino/gabarit'
import { routes } from './app.routes'
import { AVAILABLE_LANGS, StaticTranslocoLoader } from './i18n/static-transloco.loader'
import { DEFAULT_LANG } from './i18n/languages'
import { registerSiteIcons } from './shared/register-icons'
import { TrailingSlashUrlSerializer } from './shared/trailing-slash-url-serializer'
import { browserReloadEnv, isChunkLoadError, reloadOnce } from './shared/chunk-reload'

// Height of the sticky header (--header-h) plus some air.
const SCROLL_OFFSET = 96

export const appConfig: ApplicationConfig = {
  providers: [
    provideZonelessChangeDetection(),
    // No incremental hydration, so no event replay: replay injects an inline script, which our CSP (script-src 'self')
    // refuses. Links are real, so a click before hydration still works.
    provideClientHydration(withNoIncrementalHydration()),
    { provide: UrlSerializer, useClass: TrailingSlashUrlSerializer },
    provideRouter(
      routes,
      withInMemoryScrolling({ scrollPositionRestoration: 'top', anchorScrolling: 'enabled' }),
      // A failed first navigation (a chunk or a translation that could not be fetched) would leave a blank page, and so
      // would a chunk missing later: load the address in full instead.
      withNavigationErrorHandler(({ url, error }) => {
        if (!inject(Router).navigated || isChunkLoadError(error))
          reloadOnce(url, browserReloadEnv())
      }),
      // Pages cross-fade (timing in styles.scss). Skipped for reduced motion and in background tabs.
      withViewTransitions({
        skipInitialTransition: true,
        onViewTransitionCreated: ({ transition }) => {
          if (document.hidden || window.matchMedia('(prefers-reduced-motion: reduce)').matches) {
            transition.skipTransition()
          }
        },
      }),
    ),
    provideTransloco({
      config: {
        availableLangs: AVAILABLE_LANGS,
        defaultLang: DEFAULT_LANG,
        reRenderOnLangChange: true,
        prodMode: true,
      },
      loader: StaticTranslocoLoader,
    }),
    provideEnvironmentInitializer(() => registerSiteIcons(inject(IconRegistry))),
    // The router's scrollTo ignores scroll-margin, so leave room for the sticky header here.
    provideEnvironmentInitializer(() => inject(ViewportScroller).setOffset([0, SCROLL_OFFSET])),
  ],
}
