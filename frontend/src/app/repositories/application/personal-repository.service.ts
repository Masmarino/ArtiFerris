import { Injectable, inject } from '@angular/core'
import { Observable } from 'rxjs'
import { RepositoryFormat, RepositorySummary, RepositoryType } from '../domain/repository.entity'
import { PERSONAL_REPOSITORY_PORT } from './personal-repository.port'

@Injectable({ providedIn: 'root' })
export class PersonalRepositoryService {
  private readonly port = inject(PERSONAL_REPOSITORY_PORT)

  hasReservedNamespace(): Observable<boolean> {
    return this.port.checkReserved()
  }

  reserve(): Observable<void> {
    return this.port.reserve()
  }

  listMyProjects(): Observable<RepositorySummary[]> {
    return this.port.listMyProjects()
  }

  createProject(
    name: string,
    format: RepositoryFormat,
    repoType: RepositoryType,
  ): Observable<RepositorySummary> {
    return this.port.createProject(name, format, repoType)
  }
}
