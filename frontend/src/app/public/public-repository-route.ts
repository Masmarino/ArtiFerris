import { ParamMap } from '@angular/router'
import { Observable } from 'rxjs'
import { RepositoriesService } from '../repositories/application/repositories.service'
import { RepositorySummary } from '../repositories/domain/repository.entity'

/** `o/:slug/:repoName…` routes carry an organization `slug`, the personal `:username/:repoName…` ones a `username`. */
export function resolvePublicRepository(
  repositories: RepositoriesService,
  params: ParamMap,
): Observable<RepositorySummary> {
  const repoName = params.get('repoName')!
  const slug = params.get('slug')
  return slug
    ? repositories.getByOrg(slug, repoName)
    : repositories.getByOwner(params.get('username')!.replace(/^@/, ''), repoName)
}

/** Absolute routerLink prefix of the public repository page, keeping the personal `@` segment as typed. */
export function publicRepositoryBasePath(params: ParamMap): string[] {
  const repoName = params.get('repoName')!
  const slug = params.get('slug')
  return slug ? ['/o', slug, repoName] : ['/' + params.get('username')!, repoName]
}
