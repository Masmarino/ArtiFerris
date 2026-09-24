import { InjectionToken } from '@angular/core'
import { Observable } from 'rxjs'
import { ApiToken, CreatedApiToken } from '../domain/api-token.entity'

/** What the application layer needs from wherever API tokens actually live — implemented by an infrastructure adapter, never by a component directly. */
export interface ApiTokenPort {
  list(): Observable<ApiToken[]>
  /** With `currentPassword` the token lasts 365 days instead of 7. */
  create(label: string, currentPassword?: string | null): Observable<CreatedApiToken>
  revoke(id: string): Observable<void>
}

export const API_TOKEN_PORT = new InjectionToken<ApiTokenPort>('ApiTokenPort')
