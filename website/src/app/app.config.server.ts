import {
  mergeApplicationConfig,
  ApplicationConfig,
  provideEnvironmentInitializer,
} from '@angular/core'
import { provideServerRendering, withRoutes } from '@angular/ssr'
import { appConfig } from './app.config'
import { serverRoutes } from './app.routes.server'
import { installSvgInnerHtml } from './ssr-svg-support'

const serverConfig: ApplicationConfig = {
  providers: [
    provideServerRendering(withRoutes(serverRoutes)),
    provideEnvironmentInitializer(installSvgInnerHtml),
  ],
}

export const config = mergeApplicationConfig(appConfig, serverConfig)
