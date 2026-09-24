import { Injectable, inject } from '@angular/core'
import { Observable } from 'rxjs'
import {
  CatalogInfo,
  CatalogQuery,
  CatalogSearchResult,
  CatalogSuggestion,
  OwnerKind,
  OwnerSummary,
  SuggestOptions,
} from '../domain/catalog.entity'
import { PUBLIC_CATALOG_PORT } from './public-catalog.port'

@Injectable({ providedIn: 'root' })
export class CatalogService {
  private readonly port = inject(PUBLIC_CATALOG_PORT)

  catalogs(): Observable<CatalogInfo[]> {
    return this.port.catalogs()
  }

  search(query: CatalogQuery): Observable<CatalogSearchResult> {
    return this.port.search(query)
  }

  suggest(text: string, options?: SuggestOptions): Observable<CatalogSuggestion[]> {
    return this.port.suggest(text, options)
  }

  owner(kind: OwnerKind, slug: string): Observable<OwnerSummary | null> {
    return this.port.owner(kind, slug)
  }
}
