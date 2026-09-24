import { Injectable, inject } from '@angular/core'
import { HttpClient } from '@angular/common/http'
import { Observable } from 'rxjs'
import { PermissionEntry, Role, UserLookup, UserPermissionEntry } from '../domain/permission.entity'
import { PermissionPort } from '../application/permission.port'
import { apiPath } from '../../shared/api-path'

@Injectable()
export class HttpPermissionAdapter implements PermissionPort {
  private readonly http = inject(HttpClient)

  list(repositoryId: string): Observable<PermissionEntry[]> {
    return this.http.get<PermissionEntry[]>(apiPath`/api/repositories/${repositoryId}/permissions`)
  }

  searchUsers(query: string): Observable<UserLookup[]> {
    return this.http.get<UserLookup[]>('/api/users/search', { params: { q: query } })
  }

  grant(repositoryId: string, userId: string, role: Role): Observable<void> {
    return this.http.put<void>(apiPath`/api/repositories/${repositoryId}/permissions/${userId}`, {
      role,
    })
  }

  revoke(repositoryId: string, userId: string): Observable<void> {
    return this.http.delete<void>(apiPath`/api/repositories/${repositoryId}/permissions/${userId}`)
  }

  listForUser(userId: string): Observable<UserPermissionEntry[]> {
    return this.http.get<UserPermissionEntry[]>(apiPath`/api/users/${userId}/permissions`)
  }
}
