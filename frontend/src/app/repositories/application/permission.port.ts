import { InjectionToken } from '@angular/core'
import { Observable } from 'rxjs'
import { PermissionEntry, Role, UserLookup, UserPermissionEntry } from '../domain/permission.entity'

export interface PermissionPort {
  list(repositoryId: string): Observable<PermissionEntry[]>
  searchUsers(query: string): Observable<UserLookup[]>
  grant(repositoryId: string, userId: string, role: Role): Observable<void>
  revoke(repositoryId: string, userId: string): Observable<void>
  listForUser(userId: string): Observable<UserPermissionEntry[]>
}

export const PERMISSION_PORT = new InjectionToken<PermissionPort>('PermissionPort')
