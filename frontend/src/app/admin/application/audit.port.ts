import { InjectionToken } from '@angular/core'
import { Observable } from 'rxjs'
import { AuditPage, AuditQuery, BlockedAccount } from '../domain/audit.entity'

export interface AuditPort {
  query(filter?: AuditQuery): Observable<AuditPage>
  blockedAccounts(): Observable<BlockedAccount[]>
  unlockUsername(username: string): Observable<void>
}

export const AUDIT_PORT = new InjectionToken<AuditPort>('AuditPort')
