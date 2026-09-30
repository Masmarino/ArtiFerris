import { TranslocoPipe } from '@jsverse/transloco'
import { ChangeDetectionStrategy, Component, computed, effect, inject } from '@angular/core'
import { rxResource, toSignal } from '@angular/core/rxjs-interop'
import { ActivatedRoute } from '@angular/router'
import { HttpErrorResponse } from '@angular/common/http'
import { Card, Spinner } from '@masmarino/gabarit'
import { RepositoriesService } from '../../repositories/application/repositories.service'
import { RepositorySummary } from '../../repositories/domain/repository.entity'
import { UsageInstructions } from '../../repositories/usage-instructions/usage-instructions'
import { PackageTree } from '../../repositories/package-tree/package-tree'
import { PublicLayout } from '../public-layout/public-layout'
import { PageTitleService } from '../../shell/page-title.service'
import { publicRepositoryBasePath, resolvePublicRepository } from '../public-repository-route'

@Component({
  selector: 'app-public-repository-page',
  standalone: true,
  imports: [TranslocoPipe, Card, UsageInstructions, PackageTree, PublicLayout, Spinner],
  templateUrl: './public-repository-page.html',
  styleUrl: './public-repository-page.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class PublicRepositoryPage {
  private readonly route = inject(ActivatedRoute)
  private readonly repositoriesService = inject(RepositoriesService)
  private readonly pageTitle = inject(PageTitleService)

  private readonly params = toSignal(this.route.paramMap, { requireSync: true })

  private readonly resource = rxResource({
    params: () => this.params(),
    stream: ({ params }) => resolvePublicRepository(this.repositoriesService, params),
  })

  readonly loading = this.resource.isLoading
  readonly repository = computed<RepositorySummary | null>(() =>
    !this.loading() && this.resource.hasValue() ? this.resource.value() : null,
  )
  readonly notFound = computed(() => {
    const error = this.resource.error()
    return !this.loading() && error instanceof HttpErrorResponse && error.status === 404
  })
  readonly loadError = computed(
    () => !this.loading() && this.resource.error() !== undefined && !this.notFound(),
  )
  // Absolute routerLink, so links do not resolve relative to this page.
  readonly basePath = computed(() => publicRepositoryBasePath(this.params()))

  constructor() {
    effect(() => {
      const repo = this.repository()
      if (repo) {
        this.pageTitle.title.set(repo.name)
      }
    })
  }
}
