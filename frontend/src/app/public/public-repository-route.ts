import { ParamMap } from '@angular/router'
import { HttpErrorResponse } from '@angular/common/http'
import { Observable, throwError } from 'rxjs'
import { RepositoriesService } from '../repositories/application/repositories.service'
import { RepositorySummary } from '../repositories/domain/repository.entity'

/** `o/:slug/:repoName…` routes carry an organization `slug`, the personal `:username/:repoName…` ones a `username`. */
export function resolvePublicRepository(
  repositories: RepositoriesService,
  params: ParamMap,
): Observable<RepositorySummary> {
  const repoName = params.get('repoName')!
  const slug = params.get('slug')
  if (slug) {
    return repositories.getByOrg(slug, repoName)
  }
  const username = params.get('username')!.replace(/^@/, '')
  // "/@/repo" names nobody: not found, without asking the server.
  return username
    ? repositories.getByOwner(username, repoName)
    : throwError(() => new HttpErrorResponse({ status: 404 }))
}

/** Absolute routerLink prefix of the public repository page, keeping the personal `@` segment as typed. */
export function publicRepositoryBasePath(params: ParamMap): string[] {
  const repoName = params.get('repoName')!
  const slug = params.get('slug')
  return slug ? ['/o', slug, repoName] : ['/' + params.get('username')!, repoName]
}
