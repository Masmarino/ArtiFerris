import {
  ChangeDetectionStrategy,
  Component,
  computed,
  effect,
  inject,
  input,
  signal,
} from '@angular/core'
import { RouterLink } from '@angular/router'
import { Button, EmptyState, Icon, Spinner } from '@masmarino/gabarit'
import { RepositoriesService } from '../application/repositories.service'
import { RepositoryPackages } from '../domain/repository.entity'
import { VulnerabilitySummaryBadge } from '../vulnerability-summary/vulnerability-summary'

function mergePages(current: RepositoryPackages, page: RepositoryPackages): RepositoryPackages {
  if (current.format === 'npm' && page.format === 'npm') {
    return {
      format: 'npm',
      packages: [...current.packages, ...page.packages],
      next_after: page.next_after,
    }
  }
  if (current.format === 'docker' && page.format === 'docker') {
    return {
      format: 'docker',
      images: [...current.images, ...page.images],
      next_after: page.next_after,
    }
  }
  return current
}

@Component({
  selector: 'app-package-tree',
  standalone: true,
  imports: [Button, EmptyState, Icon, RouterLink, Spinner, VulnerabilitySummaryBadge],
  templateUrl: './package-tree.html',
  styleUrl: './package-tree.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class PackageTree {
  private readonly repositoriesService = inject(RepositoriesService)

  readonly repositoryId = input.required<string>()
  /** Overrides the link prefix — defaults to `/repositories/:id`. The public repository view
   * (#72) passes `['/' + rawUsernameSegment, repoName]` instead, since it has no repository id
   * in its own URL. */
  readonly basePath = input<string[] | null>(null)

  readonly tree = signal<RepositoryPackages | null>(null)
  readonly loading = signal(true)
  readonly error = signal<string | null>(null)
  readonly loadingMore = signal(false)
  readonly loadMoreError = signal(false)
  readonly hasMore = computed(() => !!this.tree()?.next_after)

  constructor() {
    effect(() => {
      this.repositoryId()
      this.reload()
    })
  }

  private reload(): void {
    this.loading.set(true)
    this.error.set(null)
    this.loadingMore.set(false)
    this.loadMoreError.set(false)
    const requestedId = this.repositoryId()
    this.repositoriesService.packages(requestedId).subscribe({
      next: (tree) => {
        if (requestedId !== this.repositoryId()) {
          return
        }
        this.tree.set(tree)
        this.loading.set(false)
      },
      error: () => {
        if (requestedId !== this.repositoryId()) {
          return
        }
        this.loading.set(false)
        this.error.set('Échec du chargement des packages.')
      },
    })
  }

  loadMore(): void {
    const current = this.tree()
    const after = current?.next_after
    if (!current || !after || this.loadingMore()) {
      return
    }
    const requestedId = this.repositoryId()
    this.loadingMore.set(true)
    this.loadMoreError.set(false)
    this.repositoriesService.packages(requestedId, after).subscribe({
      next: (page) => {
        if (requestedId !== this.repositoryId()) {
          return
        }
        this.tree.set(mergePages(current, page))
        this.loadingMore.set(false)
      },
      error: () => {
        if (requestedId !== this.repositoryId()) {
          return
        }
        this.loadingMore.set(false)
        this.loadMoreError.set(true)
      },
    })
  }

  private readonly effectiveBasePath = computed(
    () => this.basePath() ?? ['/repositories', this.repositoryId()],
  )

  readonly npmRows = computed(() => {
    const t = this.tree()
    if (!t || t.format !== 'npm') {
      return []
    }
    return t.packages.map((pkg) => ({
      pkg,
      link: [...this.effectiveBasePath(), 'packages', 'npm', pkg.name],
    }))
  })

  readonly dockerRows = computed(() => {
    const t = this.tree()
    if (!t || t.format !== 'docker') {
      return []
    }
    return t.images.map((image) => ({
      image,
      link: [...this.effectiveBasePath(), 'packages', 'docker', image.image_name],
    }))
  })
}
