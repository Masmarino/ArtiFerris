import {
  ApplicationConfig,
  LOCALE_ID,
  inject,
  isDevMode,
  provideAppInitializer,
  provideZonelessChangeDetection,
} from '@angular/core'
import { provideRouter } from '@angular/router'
import { provideHttpClient, withInterceptors } from '@angular/common/http'
import { TranslocoService, provideTransloco } from '@jsverse/transloco'
import { firstValueFrom } from 'rxjs'

import { routes } from './app.routes'
import { authInterceptor } from './auth/auth.interceptor'
import { TranslocoHttpLoader } from './transloco-loader'
import { registerLocaleData } from '@angular/common'
import localeFr from '@angular/common/locales/fr'
import { apiTokenProviders } from './tokens/infrastructure/api-token.providers'
import { authProviders } from './auth/infrastructure/auth.providers'
import { mfaProviders } from './account/infrastructure/mfa.providers'
import { userProviders } from './users/infrastructure/user.providers'
import { repositoryProviders } from './repositories/infrastructure/repository.providers'
import { adminProviders } from './admin/infrastructure/admin.providers'
import { meProviders } from './shell/infrastructure/me.providers'
import { versionProviders } from './shell/infrastructure/version.providers'
import { readableCatalogProviders } from './shell/infrastructure/readable-catalog.providers'
import { organizationsProviders } from './admin/infrastructure/organizations.providers'
import { organizationMembersProviders } from './admin/infrastructure/organization-members.providers'
import { catalogProviders } from './public/catalog/infrastructure/catalog.providers'
import { provideArtiferrisIcons } from './shared/register-icons'
import { provideTranslator } from './shared/i18n/translator'
registerLocaleData(localeFr)

export const appConfig: ApplicationConfig = {
  providers: [
    provideZonelessChangeDetection(),
    provideArtiferrisIcons(),
    provideRouter(routes),
    provideHttpClient(withInterceptors([authInterceptor])),
    {
      provide: LOCALE_ID,
      useValue: 'fr-FR',
    },
    provideTransloco({
      config: {
        availableLangs: ['fr'],
        defaultLang: 'fr',
        reRenderOnLangChange: true,
        prodMode: !isDevMode(),
      },
      loader: TranslocoHttpLoader,
    }),
    provideTranslator(),
    // Templates read translations through the pipe, but TypeScript code (error messages, labels
    // computed in components) calls translate() synchronously, so the active language must be
    // loaded before the first component is created.
    provideAppInitializer(() => {
      const transloco = inject(TranslocoService)
      return firstValueFrom(transloco.load(transloco.getActiveLang()), { defaultValue: undefined })
    }),
    // Feature port -> adapter bindings (hexagonal architecture) — each
    // feature owns its own providers array; this just spreads them in.
    ...apiTokenProviders,
    ...authProviders,
    ...mfaProviders,
    ...userProviders,
    ...organizationsProviders,
    ...organizationMembersProviders,
    ...repositoryProviders,
    ...catalogProviders,
    ...adminProviders,
    ...meProviders,
    ...versionProviders,
    ...readableCatalogProviders,
  ],
}
