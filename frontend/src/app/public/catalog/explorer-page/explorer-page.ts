import { t } from '../../../shared/i18n/translator'
import { TranslocoPipe } from '@jsverse/transloco'
import { ChangeDetectionStrategy, Component, computed, inject } from '@angular/core'
import { rxResource, toSignal } from '@angular/core/rxjs-interop'
import { ActivatedRoute, RouterLink } from '@angular/router'
import { Button, Card, Spinner } from '@masmarino/gabarit'
import { PageTitleService } from '../../../shell/page-title.service'
import { PublicLayout } from '../../public-layout/public-layout'
import { CatalogService } from '../application/catalog.service'
import { CATALOG_OVERLOAD } from '../domain/catalog-overload'
import { overloadMessage } from '../../../shared/api-error'
import { CatalogResultCard } from '../catalog-result-card/catalog-result-card'
import { CatalogSearch } from '../catalog-search/catalog-search'
import { parseFormat, parsePage, parseSort } from '../domain/catalog-params'
import { CatalogInfo, CatalogQuery } from '../domain/catalog.entity'

const POPULAR_QUERY: CatalogQuery = { sort: 'popular', perPage: 5 }

@Component({
  selector: 'app-explorer-page',
  standalone: true,
  imports: [
    TranslocoPipe,
    Button,
    Card,
    CatalogResultCard,
    CatalogSearch,
    PublicLayout,
    RouterLink,
    Spinner,
  ],
  templateUrl: './explorer-page.html',
  styleUrl: './explorer-page.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ExplorerPage {
  private readonly service = inject(CatalogService)
  private readonly queryParams = toSignal(inject(ActivatedRoute).queryParamMap, {
    requireSync: true,
  })
  private readonly catalogResource = rxResource({ stream: () => this.service.catalogs() })

  private readonly landing = computed(() => {
    const params = this.queryParams()
    return (
      !(params.get('q') ?? '').trim() &&
      !parseFormat(params.get('format')) &&
      parseSort(params.get('sort')) !== 'popular' &&
      parsePage(params.get('page')) === 1
    )
  })
  private readonly popularResource = rxResource({
    params: () => (this.landing() ? POPULAR_QUERY : undefined),
    stream: ({ params }) => this.service.search(params),
  })

  readonly loading = this.catalogResource.isLoading
  readonly failed = computed(() => !this.loading() && this.catalogResource.error() !== undefined)
  readonly failedMessage = computed(
    () =>
      overloadMessage(this.catalogResource.error(), CATALOG_OVERLOAD) ??
      t('catalog.explorer.errors.catalogs'),
  )
  readonly catalogs = computed(() =>
    !this.loading() && this.catalogResource.hasValue() ? this.catalogResource.value() : [],
  )

  readonly popularLoading = this.popularResource.isLoading
  readonly popularFailed = computed(
    () => !this.popularLoading() && this.popularResource.error() !== undefined,
  )
  readonly popularFailedMessage = computed(
    () =>
      overloadMessage(this.popularResource.error(), CATALOG_OVERLOAD) ??
      t('catalog.explorer.errors.popular'),
  )
  readonly popularEntries = computed(() =>
    !this.popularLoading() && this.popularResource.hasValue()
      ? this.popularResource.value().items
      : [],
  )
  readonly showPopular = computed(
    () =>
      this.landing() &&
      (this.popularLoading() ||
        this.popularFailed() ||
        this.popularEntries().some((entry) => entry.downloads_7d > 0)),
  )

  constructor() {
    inject(PageTitleService).title.set(t('nav.explorer'))
  }

  retry(): void {
    this.catalogResource.reload()
  }

  retryPopular(): void {
    this.popularResource.reload()
  }

  countLabel(catalog: CatalogInfo): string {
    const noun = catalog.format === 'docker' ? 'catalog.counts.image' : 'catalog.counts.package'
    return t(catalog.entry_count > 1 ? `${noun}_other` : `${noun}_one`, {
      count: catalog.entry_count,
    })
  }
}
