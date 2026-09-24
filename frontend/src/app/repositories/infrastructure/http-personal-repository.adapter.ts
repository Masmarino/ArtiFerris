import { Injectable, inject } from '@angular/core'
import { HttpClient, HttpErrorResponse } from '@angular/common/http'
import { Observable, catchError, map, of, throwError } from 'rxjs'
import { RepositoryFormat, RepositorySummary, RepositoryType } from '../domain/repository.entity'
import { PersonalRepositoryPort } from '../application/personal-repository.port'

@Injectable()
export class HttpPersonalRepositoryAdapter implements PersonalRepositoryPort {
  private readonly http = inject(HttpClient)

  checkReserved(): Observable<boolean> {
    return this.http.get<void>('/api/me/repository').pipe(
      map(() => true),
      // A 404 is this endpoint's normal "no personal namespace yet" answer, not a failure.
      catchError((err: unknown) => {
        if (err instanceof HttpErrorResponse && err.status === 404) {
          return of(false)
        }
        return throwError(() => err)
      }),
    )
  }

  reserve(): Observable<void> {
    return this.http.post<void>('/api/me/repository', {})
  }

  listMyProjects(): Observable<RepositorySummary[]> {
    return this.http.get<RepositorySummary[]>('/api/me/repository/projects')
  }

  createProject(
    name: string,
    format: RepositoryFormat,
    repoType: RepositoryType,
  ): Observable<RepositorySummary> {
    return this.http.post<RepositorySummary>('/api/me/repository/projects', {
      name,
      format,
      repo_type: repoType,
    })
  }
}
