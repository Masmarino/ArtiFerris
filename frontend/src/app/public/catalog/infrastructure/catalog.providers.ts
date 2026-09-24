import { Provider } from '@angular/core'
import { PUBLIC_CATALOG_PORT } from '../application/public-catalog.port'
import { HttpPublicCatalogAdapter } from './http-public-catalog.adapter'

export const catalogProviders: Provider[] = [
  { provide: PUBLIC_CATALOG_PORT, useClass: HttpPublicCatalogAdapter },
]
