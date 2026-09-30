import { Provider } from '@angular/core'
import { of } from 'rxjs'
import { CatalogService } from '../application/catalog.service'

export const NO_SUGGESTIONS: Provider = {
  provide: CatalogService,
  useValue: { suggest: () => of([]) },
}
