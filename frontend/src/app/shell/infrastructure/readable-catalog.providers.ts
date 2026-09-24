import { Provider } from '@angular/core'
import { READABLE_CATALOG_PORT } from '../application/readable-catalog.port'
import { HttpReadableCatalogAdapter } from './http-readable-catalog.adapter'

export const readableCatalogProviders: Provider[] = [
  { provide: READABLE_CATALOG_PORT, useClass: HttpReadableCatalogAdapter },
]
