import { Injectable, inject } from '@angular/core'
import { HttpClient } from '@angular/common/http'
import { Observable } from 'rxjs'
import { AuditPage, AuditQuery, BlockedAccount } from '../domain/audit.entity'
import { AuditPort } from '../application/audit.port'
import { apiPath } from '../../shared/api-path'

@Injectable()
export class HttpAuditAdapter implements AuditPort {
  private readonly http = inject(HttpClient)

  query(filter: AuditQuery = {}): Observable<AuditPage> {
    const params: Record<string, string> = {}
    for (const [key, value] of Object.entries(filter)) {
      if (value) {
        params[key] = String(value)
      }
    }
    return this.http.get<AuditPage>('/api/audit/events', { params })
  }

  blockedAccounts(): Observable<BlockedAccount[]> {
    return this.http.get<BlockedAccount[]>('/api/admin/security/blocked')
  }

  unlockUsername(username: string): Observable<void> {
    return this.http.delete<void>(apiPath`/api/admin/login-throttle/${username}`)
  }
}
