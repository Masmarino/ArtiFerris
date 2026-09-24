import { Injectable, inject } from '@angular/core'
import { Observable } from 'rxjs'
import {
  ReadableCatalogQuery,
  ReadableCatalogSearchResult,
} from '../domain/readable-catalog.entity'
import { READABLE_CATALOG_PORT } from './readable-catalog.port'

@Injectable({ providedIn: 'root' })
export class ReadableCatalogService {
  private readonly port = inject(READABLE_CATALOG_PORT)

  search(query: ReadableCatalogQuery): Observable<ReadableCatalogSearchResult> {
    return this.port.search(query)
  }
}
