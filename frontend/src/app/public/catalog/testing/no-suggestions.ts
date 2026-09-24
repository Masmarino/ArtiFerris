import { Provider } from '@angular/core'
import { of } from 'rxjs'
import { CatalogService } from '../application/catalog.service'

/** For pages that only embed the header search box: it suggests nothing. */
export const NO_SUGGESTIONS: Provider = {
  provide: CatalogService,
  useValue: { suggest: () => of([]) },
}
