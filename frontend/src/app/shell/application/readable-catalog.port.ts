import { InjectionToken } from '@angular/core'
import { Observable } from 'rxjs'
import {
  ReadableCatalogQuery,
  ReadableCatalogSearchResult,
} from '../domain/readable-catalog.entity'

export interface ReadableCatalogPort {
  search(query: ReadableCatalogQuery): Observable<ReadableCatalogSearchResult>
}

export const READABLE_CATALOG_PORT = new InjectionToken<ReadableCatalogPort>('ReadableCatalogPort')
