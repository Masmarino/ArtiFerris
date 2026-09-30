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
import { provideTransloco } from '@jsverse/transloco'

import { routes } from './app.routes'
import { authInterceptor } from './auth/auth.interceptor'
import { TranslocoHttpLoader } from './transloco-loader'
import './shared/i18n/locale-data'
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
import { LanguageService } from './shared/i18n/language.service'
import {
  FALLBACK_LANGUAGE,
  LANGUAGE_LOCALES,
  SUPPORTED_LANGUAGES,
  detectBrowserLanguage,
} from './shared/i18n/languages'

export const appConfig: ApplicationConfig = {
  providers: [
    provideZonelessChangeDetection(),
    provideArtiferrisIcons(),
    provideRouter(routes),
    provideHttpClient(withInterceptors([authInterceptor])),
    { provide: LOCALE_ID, useFactory: () => LANGUAGE_LOCALES[detectBrowserLanguage()] },
    provideTransloco({
      config: {
        availableLangs: SUPPORTED_LANGUAGES,
        defaultLang: FALLBACK_LANGUAGE,
        reRenderOnLangChange: true,
        prodMode: !isDevMode(),
      },
      loader: TranslocoHttpLoader,
    }),
    provideTranslator(),
    // Templates read translations through the pipe, but TypeScript code (error messages, labels
    // computed in components) calls translate() synchronously, so the active language must be
    // loaded before the first component is created.
    provideAppInitializer(() => inject(LanguageService).use(detectBrowserLanguage())),
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
