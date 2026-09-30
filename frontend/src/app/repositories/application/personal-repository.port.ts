import { InjectionToken } from '@angular/core'
import { Observable } from 'rxjs'
import { RepositoryFormat, RepositorySummary, RepositoryType } from '../domain/repository.entity'

export interface PersonalRepositoryPort {
  /** `false` means the caller has no personal namespace yet — not a failure. */
  checkReserved(): Observable<boolean>

  reserve(): Observable<void>

  listMyProjects(): Observable<RepositorySummary[]>

  createProject(
    name: string,
    format: RepositoryFormat,
    repoType: RepositoryType,
  ): Observable<RepositorySummary>
}

export const PERSONAL_REPOSITORY_PORT = new InjectionToken<PersonalRepositoryPort>(
  'PersonalRepositoryPort',
)
