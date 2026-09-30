import { InjectionToken } from '@angular/core'
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

export interface PublicCatalogPort {
  catalogs(): Observable<CatalogInfo[]>

  search(query: CatalogQuery): Observable<CatalogSearchResult>

  /** The best names matching the text, narrowed by the server to the given format and owner. Fails for text outside 2 to 100 characters or when throttled. */
  suggest(text: string, options?: SuggestOptions): Observable<CatalogSuggestion[]>

  /** Emits `null` when the owner has nothing public, whether or not the account exists. */
  owner(kind: OwnerKind, slug: string): Observable<OwnerSummary | null>
}

export const PUBLIC_CATALOG_PORT = new InjectionToken<PublicCatalogPort>('PublicCatalogPort')
