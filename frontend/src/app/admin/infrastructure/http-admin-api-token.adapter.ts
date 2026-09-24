import { Injectable, inject } from '@angular/core'
import { HttpClient } from '@angular/common/http'
import { Observable } from 'rxjs'
import { ADMIN_TOKEN_PAGE_LIMIT, AdminApiToken } from '../domain/admin-api-token.entity'
import { AdminApiTokenPort } from '../application/admin-api-token.port'
import { apiPath } from '../../shared/api-path'

function listParams(organizationId?: string): Record<string, string> {
  const limit = String(ADMIN_TOKEN_PAGE_LIMIT)
  return organizationId ? { organization_id: organizationId, limit } : { limit }
}

@Injectable()
export class HttpAdminApiTokenAdapter implements AdminApiTokenPort {
  private readonly http = inject(HttpClient)

  list(organizationId?: string): Observable<AdminApiToken[]> {
    return this.http.get<AdminApiToken[]>('/api/admin/tokens', {
      params: listParams(organizationId),
    })
  }

  revoke(id: string): Observable<void> {
    return this.http.delete<void>(apiPath`/api/admin/tokens/${id}`)
  }
}
