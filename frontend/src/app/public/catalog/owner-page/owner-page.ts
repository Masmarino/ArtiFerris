import { ChangeDetectionStrategy, Component, computed, effect, inject } from '@angular/core'
import { rxResource, toSignal } from '@angular/core/rxjs-interop'
import { ActivatedRoute, RouterLink } from '@angular/router'
import { Button, EmptyState, Spinner } from '@masmarino/gabarit'
import { PageTitleService } from '../../../shell/page-title.service'
import { PublicLayout } from '../../public-layout/public-layout'
import { CatalogService } from '../application/catalog.service'
import { CATALOG_OVERLOAD } from '../domain/catalog-overload'
import { overloadMessage } from '../../../shared/api-error'
import { CatalogSearch } from '../catalog-search/catalog-search'
import { OwnerRef } from '../domain/catalog.entity'
import { ownerCountsLabel } from '../domain/owner-counts'

/** Serves both `/@:username` (param `username`) and `/o/:slug` (param `slug`). */
@Component({
  selector: 'app-owner-page',
  standalone: true,
  imports: [Button, CatalogSearch, EmptyState, PublicLayout, RouterLink, Spinner],
  templateUrl: './owner-page.html',
  styleUrl: './owner-page.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class OwnerPage {
  private readonly service = inject(CatalogService)
  private readonly pathParams = toSignal(inject(ActivatedRoute).paramMap, { requireSync: true })

  private readonly requested = computed<OwnerRef>(() => {
    const params = this.pathParams()
    const username = params.get('username')
    return username
      ? { kind: 'personal', slug: username }
      : { kind: 'organization', slug: params.get('slug') ?? '' }
  })

  private readonly resource = rxResource({
    params: () => this.requested(),
    stream: ({ params }) => this.service.owner(params.kind, params.slug),
  })

  readonly loading = this.resource.isLoading
  readonly failed = computed(() => !this.loading() && this.resource.error() !== undefined)
  private readonly loaded = computed(() =>
    !this.loading() && this.resource.hasValue() ? this.resource.value() : undefined,
  )
  readonly notFound = computed(() => this.loaded() === null)
  readonly summary = computed(() => this.loaded() ?? null)
  readonly ownerRef = computed<OwnerRef | null>(() => {
    const summary = this.summary()
    return summary && { kind: summary.kind, slug: summary.slug }
  })
  readonly counts = computed(() => {
    const summary = this.summary()
    return summary ? ownerCountsLabel(summary) : ''
  })
  readonly errorMessage = computed(() => {
    return (
      overloadMessage(this.resource.error(), CATALOG_OVERLOAD) ??
      'Échec du chargement du profil. Vérifiez votre connexion puis réessayez.'
    )
  })

  constructor() {
    const pageTitle = inject(PageTitleService)
    effect(() => pageTitle.title.set(this.summary()?.display_name ?? 'Propriétaire'))
  }

  retry(): void {
    this.resource.reload()
  }
}
