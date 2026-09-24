import { Injectable, inject } from '@angular/core'
import { HttpClient, HttpParams } from '@angular/common/http'
import { Observable } from 'rxjs'
import {
  ReadableCatalogQuery,
  ReadableCatalogSearchResult,
} from '../domain/readable-catalog.entity'
import { ReadableCatalogPort } from '../application/readable-catalog.port'

@Injectable()
export class HttpReadableCatalogAdapter implements ReadableCatalogPort {
  private readonly http = inject(HttpClient)

  search(query: ReadableCatalogQuery): Observable<ReadableCatalogSearchResult> {
    let params = new HttpParams()
    const text = query.q?.trim()
    if (text) {
      params = params.set('q', text)
    }
    if (query.format) {
      params = params.set('format', query.format)
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
    return this.http.get<ReadableCatalogSearchResult>('/api/search', { params })
  }
}
