import { Injectable, inject } from '@angular/core'
import { HttpClient } from '@angular/common/http'
import { Observable } from 'rxjs'
import { ApiToken, CreatedApiToken } from '../domain/api-token.entity'
import { ApiTokenPort } from '../application/api-token.port'
import { apiPath } from '../../shared/api-path'

@Injectable()
export class HttpApiTokenAdapter implements ApiTokenPort {
  private readonly http = inject(HttpClient)

  list(): Observable<ApiToken[]> {
    return this.http.get<ApiToken[]>('/api/tokens')
  }

  create(label: string, currentPassword: string | null = null): Observable<CreatedApiToken> {
    return this.http.post<CreatedApiToken>(
      '/api/tokens',
      currentPassword ? { label, current_password: currentPassword } : { label },
    )
  }

  revoke(id: string): Observable<void> {
    return this.http.delete<void>(apiPath`/api/tokens/${id}`)
  }
}
