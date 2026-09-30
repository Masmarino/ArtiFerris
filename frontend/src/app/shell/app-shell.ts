import { t } from '../shared/i18n/translator'
import { LanguageService } from '../shared/i18n/language.service'
import { TranslocoPipe } from '@jsverse/transloco'
import {
  ChangeDetectionStrategy,
  Component,
  ElementRef,
  HostListener,
  OnInit,
  computed,
  effect,
  untracked,
  inject,
  signal,
  viewChild,
} from '@angular/core'
import { HttpErrorResponse } from '@angular/common/http'
import { takeUntilDestroyed, toSignal } from '@angular/core/rxjs-interop'
import {
  ActivatedRoute,
  NavigationEnd,
  Router,
  RouterLink,
  RouterLinkActive,
  RouterOutlet,
} from '@angular/router'
import {
  Subject,
  catchError,
  debounce,
  distinctUntilChanged,
  filter,
  map,
  of,
  switchMap,
  timer,
} from 'rxjs'
import {
  AppShell as GbtAppShell,
  Button,
  Icon,
  SearchBar,
  SearchResultCategory,
  Spinner,
  Toaster,
} from '@masmarino/gabarit'
import { AuthService } from '../auth/application/auth.service'
import { MeService } from './application/me.service'
import { ReadableCatalogService } from './application/readable-catalog.service'
import { ReadableCatalogEntry } from './domain/readable-catalog.entity'
import { CatalogFormat } from '../public/catalog/domain/catalog.entity'
import { CATALOGS } from '../public/catalog/domain/catalog.registry'
import { VersionService } from './application/version.service'
import { PageTitleService } from './page-title.service'
import { ConfirmHost } from '../shared/confirm-host/confirm-host'
import { RepositoriesService } from '../repositories/application/repositories.service'
import { RepositorySummary } from '../repositories/domain/repository.entity'
import { UsersService } from '../users/application/users.service'
import { UserSummary } from '../users/domain/user.entity'
import { formatResultsAnnouncement } from '../shared/format'
import { ToastService } from '../shared/toast.service'

interface NavItem {
  action: string
  icon: string
  text: string
  link: string
  children?: NavItem[]
  /** '/' would contain every route under non-exact matching, so it always lit up. */
  exact?: boolean
}

type SearchResult =
  | { kind: 'repository' | 'user'; id: string; label: string }
  | { kind: 'package'; id: string; label: string; format: CatalogFormat; name: string }

const PACKAGE_SEARCH_MIN_LENGTH = 2
const PACKAGE_SEARCH_DEBOUNCE_MS = 250
const PACKAGE_SEARCH_LIMIT = 5

function packageLabel(entry: ReadableCatalogEntry): string {
  const format = CATALOGS.find((catalog) => catalog.format === entry.kind)?.label ?? entry.kind
  const label = `${entry.name} (${format})`
  return entry.repository.repo_type === 'proxy'
    ? `${label} — ${t('shell.search.proxyCache')}`
    : label
}

const SUPER_ADMIN_ONLY_ACTIONS = new Set(['admin'])
const STAFF_ONLY_ACTIONS = new Set(['users'])

@Component({
  selector: 'app-shell',
  standalone: true,
  imports: [
    TranslocoPipe,
    RouterOutlet,
    RouterLink,
    RouterLinkActive,
    Icon,
    SearchBar,
    Spinner,
    Toaster,
    Button,
    GbtAppShell,
    ConfirmHost,
  ],
  templateUrl: './app-shell.html',
  styleUrl: './app-shell.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class AppShell implements OnInit {
  private readonly auth = inject(AuthService)
  private readonly router = inject(Router)
  private readonly activatedRoute = inject(ActivatedRoute)
  private readonly repositoriesService = inject(RepositoriesService)
  private readonly usersService = inject(UsersService)
  private readonly readableCatalog = inject(ReadableCatalogService)
  readonly me = inject(MeService)
  readonly version = inject(VersionService)
  readonly pageTitle = inject(PageTitleService)
  readonly toastService = inject(ToastService)
  private readonly languageService = inject(LanguageService)
  readonly viewGeneration = signal(0)

  readonly isLoading = signal(true)
  readonly loadFailed = signal(false)
  readonly userMenuOpen = signal(false)
  readonly navCollapsed = signal(false)
  private readonly menuManualOverrides = signal<Record<string, boolean>>({})

  private readonly repositories = signal<RepositorySummary[]>([])
  private readonly users = signal<UserSummary[]>([])
  private readonly packages = signal<ReadableCatalogEntry[]>([])
  private readonly packageQueries = new Subject<string>()
  readonly searchQuery = signal('')
  readonly searchFailed = signal(false)

  // Whoever can reach /users can also search it.
  private readonly canSeeUsers = computed(
    () => this.me.isSuperAdmin() || this.me.isOrganizationAdmin(),
  )

  readonly searchResults = computed<SearchResultCategory<SearchResult>[]>(() => {
    const query = this.searchQuery().trim().toLowerCase()
    if (!query) {
      return []
    }
    const matchingRepositories: SearchResult[] = this.repositories()
      .filter((repository) => repository.name.toLowerCase().includes(query))
      .map((repository) => ({ kind: 'repository', id: repository.id, label: repository.name }))
    const matchingUsers: SearchResult[] = this.users()
      .filter((user) => user.username.toLowerCase().includes(query))
      .map((user) => ({ kind: 'user', id: user.id, label: user.username }))

    const categories: SearchResultCategory<SearchResult>[] = [
      { label: t('nav.repositories'), icon: 'package', items: matchingRepositories },
    ]
    if (this.canSeeUsers()) {
      categories.push({ label: t('nav.users'), icon: 'user', items: matchingUsers })
    }
    const matchingPackages = this.packages().map((entry): SearchResult => ({
      kind: 'package',
      id: entry.repository.id,
      format: entry.kind,
      name: entry.name,
      label: packageLabel(entry),
    }))
    if (matchingPackages.length > 0) {
      categories.push({
        label: t('shell.search.packagesAndImages'),
        icon: 'package',
        items: matchingPackages,
      })
    }
    return categories
  })

  readonly searchResultLabel = (item: SearchResult): string => item.label
  readonly resultsAnnouncement = formatResultsAnnouncement

  private readonly userMenu = viewChild<ElementRef<HTMLElement>>('userMenu')

  private readonly currentUrl = toSignal(
    this.router.events.pipe(
      filter((event) => event instanceof NavigationEnd),
      map(() => this.router.url),
    ),
    { initialValue: this.router.url },
  )

  // Starts empty: calling deepestRouteTitle() here is too early.
  private readonly routeTitle = toSignal(
    this.router.events.pipe(
      filter((event) => event instanceof NavigationEnd),
      map(() => this.deepestRouteTitle()),
    ),
    { initialValue: '' },
  )

  readonly navItems = computed<NavItem[]>(() => {
    const items: NavItem[] = [
      { action: 'explorer', icon: 'compass', text: t('nav.explorer'), link: '/', exact: true },
      {
        action: 'repositories',
        icon: 'package',
        text: t('nav.repositories'),
        link: '/repositories',
      },
      {
        action: 'my-repository',
        icon: 'user',
        text: t('nav.myRepository'),
        link: '/my-repository',
      },
      { action: 'users', icon: 'users', text: t('nav.users'), link: '/users' },
      {
        action: 'admin',
        icon: 'layout-dashboard',
        text: t('nav.administration'),
        link: '/admin',
        children: [
          {
            action: 'organizations',
            icon: 'server',
            text: t('nav.organizations'),
            link: '/admin/organizations',
          },
          { action: 'export', icon: 'download', text: t('nav.export'), link: '/admin/export' },
          { action: 'health', icon: 'activity', text: t('nav.health'), link: '/admin/health' },
        ],
      },
    ]
    if (this.me.isSuperAdmin()) {
      return items
    }
    let filtered = items.filter((item) => !SUPER_ADMIN_ONLY_ACTIONS.has(item.action))
    if (!this.me.isOrganizationAdmin()) {
      filtered = filtered.filter((item) => !STAFF_ONLY_ACTIONS.has(item.action))
    }
    const organizationId = this.me.organizationId()
    if (this.me.isOrganizationAdmin() && organizationId) {
      filtered.push({
        action: 'organization',
        icon: 'layout-dashboard',
        text: t('nav.administration'),
        link: `/admin/organizations/${organizationId}`,
      })
    }
    return filtered
  })

  constructor() {
    effect(() => this.pageTitle.title.set(this.routeTitle()))
    let firstRun = true
    effect(() => {
      this.languageService.language()
      untracked(() => {
        if (firstRun) {
          firstRun = false
          return
        }
        this.viewGeneration.update((generation) => generation + 1)
        const title = this.deepestRouteTitle()
        if (title) {
          this.pageTitle.title.set(title)
        }
      })
    })
    this.packageQueries
      .pipe(
        // A too-short query clears at once; a longer one waits for a pause.
        debounce((query) =>
          query.length < PACKAGE_SEARCH_MIN_LENGTH ? of(0) : timer(PACKAGE_SEARCH_DEBOUNCE_MS),
        ),
        distinctUntilChanged(),
        // switchMap drops the in-flight request, so a slow answer never overwrites a newer one.
        switchMap((query) =>
          query.length < PACKAGE_SEARCH_MIN_LENGTH
            ? of([])
            : this.readableCatalog.search({ q: query, perPage: PACKAGE_SEARCH_LIMIT }).pipe(
                map((result) => result.items),
                catchError(() => {
                  this.searchFailed.set(true)
                  return of([])
                }),
              ),
        ),
        takeUntilDestroyed(),
      )
      .subscribe((entries) => this.packages.set(entries))
  }

  ngOnInit(): void {
    this.version.load()
    this.loadMe()
  }

  retryLoad(): void {
    this.loadMe()
  }

  private loadMe(): void {
    this.isLoading.set(true)
    this.loadFailed.set(false)
    this.me.load().subscribe({
      next: () => {
        this.isLoading.set(false)
        this.refreshSearchData()
      },
      error: (error: unknown) => {
        this.isLoading.set(false)
        // Only a refused session ends it; a 5xx or network error keeps the token.
        if (error instanceof HttpErrorResponse && (error.status === 401 || error.status === 403)) {
          this.auth.logout()
          this.router.navigateByUrl('/login')
        } else {
          this.loadFailed.set(true)
        }
      },
    })
  }

  // Refresh when a search starts, to catch changes made elsewhere.
  onSearchInput(query: string): void {
    if ((!this.searchQuery() && query) || this.searchFailed()) {
      this.refreshSearchData()
    }
    this.setSearchQuery(query)
  }

  private setSearchQuery(query: string): void {
    this.searchQuery.set(query)
    this.packageQueries.next(query.trim())
  }

  private refreshSearchData(): void {
    this.searchFailed.set(false)
    const failed = () => this.searchFailed.set(true)
    this.repositoriesService.list({ forceRefresh: true }).subscribe({
      next: (repositories) => this.repositories.set(repositories),
      error: failed,
    })
    if (this.canSeeUsers()) {
      this.usersService.list({ forceRefresh: true }).subscribe({
        next: (users) => this.users.set(users),
        error: failed,
      })
    }
  }

  onSelectResult(item: SearchResult): void {
    this.setSearchQuery('')
    if (item.kind === 'package') {
      this.router.navigate(['/repositories', item.id, 'packages', item.format, item.name])
      return
    }
    this.router.navigateByUrl(
      item.kind === 'repository' ? `/repositories/${item.id}` : `/users/${item.id}`,
    )
  }

  private deepestRouteTitle(): string {
    let route = this.activatedRoute
    while (route.firstChild) {
      route = route.firstChild
    }
    const titleKey = route.snapshot.data['titleKey'] as string | undefined
    return titleKey ? t(titleKey) : ''
  }

  logout(): void {
    this.userMenuOpen.set(false)
    this.auth.logout()
    this.router.navigateByUrl('/login')
  }

  toggleUserMenu(): void {
    this.userMenuOpen.update((open) => !open)
  }

  isMenuOpen(item: NavItem): boolean {
    return this.menuManualOverrides()[item.action] ?? this.currentUrl().startsWith(item.link)
  }

  toggleMenu(item: NavItem): void {
    this.menuManualOverrides.update((overrides) => ({
      ...overrides,
      [item.action]: !this.isMenuOpen(item),
    }))
  }

  @HostListener('document:click', ['$event'])
  protected onDocumentClick(event: MouseEvent): void {
    const menuEl = this.userMenu()?.nativeElement
    if (this.userMenuOpen() && menuEl && !menuEl.contains(event.target as Node)) {
      this.userMenuOpen.set(false)
    }
  }

  @HostListener('document:keydown.escape')
  protected onEscape(): void {
    this.userMenuOpen.set(false)
  }
}
