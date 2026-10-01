import { t } from '../../../shared/i18n/translator'
import { TranslocoPipe } from '@jsverse/transloco'
import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  ElementRef,
  computed,
  effect,
  inject,
  input,
  signal,
  untracked,
  viewChild,
} from '@angular/core'
import { rxResource, toSignal } from '@angular/core/rxjs-interop'
import { ActivatedRoute, Params, Router } from '@angular/router'
import {
  Button,
  EmptyState,
  SegmentedControl,
  SegmentedControlOption,
  Spinner,
} from '@masmarino/gabarit'
import { formatResultsAnnouncement } from '../../../shared/format'
import { CatalogService } from '../application/catalog.service'
import { CATALOG_OVERLOAD } from '../domain/catalog-overload'
import { overloadMessage } from '../../../shared/api-error'
import {
  CatalogFormat,
  CatalogInfo,
  CatalogQuery,
  CatalogSort,
  OwnerRef,
} from '../domain/catalog.entity'
import { parseFormat, parsePage, parseSort } from '../domain/catalog-params'
import { CATALOGS } from '../domain/catalog.registry'
import { CatalogResultCard } from '../catalog-result-card/catalog-result-card'
import { SuggestSearchBox } from '../suggest-search-box/suggest-search-box'

const DEBOUNCE_MS = 300
const MAX_QUERY_LENGTH = 100

@Component({
  selector: 'app-catalog-search',
  standalone: true,
  imports: [
    TranslocoPipe,
    Button,
    CatalogResultCard,
    EmptyState,
    SegmentedControl,
    Spinner,
    SuggestSearchBox,
  ],
  templateUrl: './catalog-search.html',
  styleUrl: './catalog-search.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class CatalogSearch {
  private readonly route = inject(ActivatedRoute)
  private readonly router = inject(Router)
  private readonly service = inject(CatalogService)
  private readonly destroyRef = inject(DestroyRef)
  private debounceTimer: ReturnType<typeof setTimeout> | null = null

  readonly format = input<CatalogFormat | null>(null)
  /** Restricts every search to this owner. Comes from the route, never the query string. */
  readonly owner = input<OwnerRef | null>(null)
  readonly catalogs = input<CatalogInfo[]>([])

  private readonly queryParams = toSignal(this.route.queryParamMap, { requireSync: true })
  private readonly urlQuery = computed(() => this.queryParams().get('q') ?? '')
  private readonly sortParam = computed(() => parseSort(this.queryParams().get('sort')))

  readonly page = computed(() => parsePage(this.queryParams().get('page')))
  readonly activeFormat = computed(
    () => this.format() ?? parseFormat(this.queryParams().get('format')),
  )
  readonly searchedText = computed(() => this.urlQuery().trim())
  readonly sort = computed<CatalogSort>(() => {
    const sort = this.sortParam()
    if (sort === 'popular') {
      return sort
    }
    return this.searchedText() ? (sort ?? 'relevance') : 'updated'
  })

  readonly text = signal(this.urlQuery())

  readonly formatOptions = computed<SegmentedControlOption[]>(() => [
    { value: '', label: t('catalog.search.sort.all') },
    ...(this.catalogs().length > 0 ? this.catalogs() : CATALOGS).map((catalog) => ({
      value: catalog.format,
      label: catalog.label,
    })),
  ])
  readonly sortOptions = computed<SegmentedControlOption[]>(() => [
    {
      value: 'relevance',
      label: t('catalog.search.sort.relevance'),
      disabled: !this.searchedText(),
    },
    { value: 'updated', label: t('catalog.search.sort.updated') },
    { value: 'popular', label: t('catalog.search.sort.popular') },
  ])

  private readonly request = computed<CatalogQuery>(() => ({
    q: this.searchedText().slice(0, MAX_QUERY_LENGTH) || undefined,
    format: this.activeFormat() ?? undefined,
    owner: this.owner() ?? undefined,
    sort: this.sortParam() ? this.sort() : undefined,
    page: this.page(),
  }))

  private readonly search = rxResource({
    params: () => this.request(),
    stream: ({ params }) => this.service.search(params),
  })

  readonly loading = this.search.isLoading
  readonly failed = computed(() => !this.loading() && this.search.error() !== undefined)
  readonly result = computed(() =>
    !this.loading() && this.search.hasValue() ? this.search.value() : undefined,
  )

  readonly exactItems = computed(
    () => this.result()?.items.filter((entry) => entry.match_kind !== 'fuzzy') ?? [],
  )
  readonly approximateItems = computed(
    () => this.result()?.items.filter((entry) => entry.match_kind === 'fuzzy') ?? [],
  )

  readonly totalPages = computed(() => {
    const result = this.result()
    return result && result.per_page > 0
      ? Math.max(1, Math.ceil(result.total / result.per_page))
      : 1
  })
  readonly resultCount = computed(() => {
    const result = this.result()
    if (!result) {
      return ''
    }
    return result.total === 0
      ? t('shell.search.noResults')
      : formatResultsAnnouncement(result.total)
  })
  readonly errorMessage = computed(() => {
    return (
      overloadMessage(this.search.error(), CATALOG_OVERLOAD) ?? t('catalog.search.errors.failed')
    )
  })

  private readonly heading = viewChild<ElementRef<HTMLElement>>('resultsHeading')

  constructor() {
    effect(() => {
      const urlQuery = this.urlQuery()
      untracked(() => {
        // Skip while the user is typing, or the URL catching up would overwrite it.
        if (this.debounceTimer === null && urlQuery !== this.text()) {
          this.text.set(urlQuery)
        }
      })
    })

    effect(() => {
      const result = this.result()
      if (result && result.total > 0 && result.items.length === 0) {
        untracked(() =>
          this.navigate({ page: this.totalPages() > 1 ? this.totalPages() : null }, true),
        )
      }
    })

    this.destroyRef.onDestroy(() => this.clearDebounce())
  }

  onTextChange(value: string): void {
    this.text.set(value)
    this.clearDebounce()
    this.debounceTimer = setTimeout(() => {
      this.debounceTimer = null
      this.pushText(true)
    }, DEBOUNCE_MS)
  }

  searchNow(): void {
    this.clearDebounce()
    this.pushText(false)
  }

  setFormat(value: string): void {
    this.navigate({ format: value || null, page: null })
  }

  setSort(value: string): void {
    this.navigate({ sort: parseSort(value), page: null })
  }

  goToPage(page: number): void {
    const target = Math.min(Math.max(page, 1), this.totalPages())
    this.navigate({ page: target > 1 ? target : null })
    this.heading()?.nativeElement.focus()
  }

  retry(): void {
    this.search.reload()
  }

  clearSearch(): void {
    this.clearDebounce()
    this.text.set('')
    this.navigate(this.format() ? { q: null, page: null } : { q: null, format: null, page: null })
  }

  private pushText(replaceUrl: boolean): void {
    this.navigate({ q: this.text().trim() || null, page: null }, replaceUrl)
  }

  private clearDebounce(): void {
    if (this.debounceTimer !== null) {
      clearTimeout(this.debounceTimer)
      this.debounceTimer = null
    }
  }

  private navigate(queryParams: Params, replaceUrl = false): void {
    void this.router.navigate([], {
      relativeTo: this.route,
      queryParams,
      queryParamsHandling: 'merge',
      replaceUrl,
    })
  }
}
