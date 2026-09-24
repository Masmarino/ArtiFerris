import { Injectable, inject } from '@angular/core'
import { HttpClient, HttpErrorResponse, HttpParams } from '@angular/common/http'
import { Observable, catchError, of, throwError } from 'rxjs'
import {
  CatalogInfo,
  CatalogQuery,
  CatalogSearchResult,
  CatalogSuggestion,
  OwnerKind,
  OwnerSummary,
  SuggestOptions,
} from '../domain/catalog.entity'
import { PublicCatalogPort } from '../application/public-catalog.port'
import { apiPath } from '../../../shared/api-path'

@Injectable()
export class HttpPublicCatalogAdapter implements PublicCatalogPort {
  private readonly http = inject(HttpClient)

  catalogs(): Observable<CatalogInfo[]> {
    return this.http.get<CatalogInfo[]>('/api/public/catalogs')
  }

  search(query: CatalogQuery): Observable<CatalogSearchResult> {
    let params = new HttpParams()
    const text = query.q?.trim()
    if (text) {
      params = params.set('q', text)
    }
    if (query.format) {
      params = params.set('format', query.format)
    }
    if (query.owner) {
      params = params.set('owner', `${query.owner.kind}:${query.owner.slug}`)
    }
    if (query.sort) {
      params = params.set('sort', query.sort)
    }
    if (query.page) {
      params = params.set('page', query.page)
    }
    if (query.perPage) {
      params = params.set('per_page', query.perPage)
    }
    return this.http.get<CatalogSearchResult>('/api/public/search', { params })
  }

  suggest(text: string, options: SuggestOptions = {}): Observable<CatalogSuggestion[]> {
    let params = new HttpParams().set('q', text.trim())
    if (options.format) {
      params = params.set('format', options.format)
    }
    if (options.owner) {
      params = params.set('owner', `${options.owner.kind}:${options.owner.slug}`)
    }
    if (options.limit !== undefined) {
      params = params.set('limit', options.limit)
    }
    return this.http.get<CatalogSuggestion[]>('/api/public/suggest', { params })
  }

  owner(kind: OwnerKind, slug: string): Observable<OwnerSummary | null> {
    return this.http
      .get<OwnerSummary>(apiPath`/api/public/owners/${kind}/${slug}`)
      .pipe(
        catchError((error: unknown) =>
          error instanceof HttpErrorResponse && error.status === 404
            ? of(null)
            : throwError(() => error),
        ),
      )
  }
}
