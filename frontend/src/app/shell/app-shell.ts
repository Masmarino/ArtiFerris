import {
  ChangeDetectionStrategy,
  Component,
  ElementRef,
  HostListener,
  OnInit,
  computed,
  effect,
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
  /** '/' would otherwise "contain" every other route under non-exact matching, always lighting up. */
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
  return entry.repository.repo_type === 'proxy' ? `${label} — cache du proxy` : label
}

// The instance-wide flat Administration menu is super-admin only.
const SUPER_ADMIN_ONLY_ACTIONS = new Set(['admin'])
// Reachable by a super-admin or an organization admin, never a plain member.
const STAFF_ONLY_ACTIONS = new Set(['users'])

@Component({
  selector: 'app-shell',
  standalone: true,
  imports: [
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

  readonly isLoading = signal(true)
  readonly loadFailed = signal(false)
  readonly userMenuOpen = signal(false)
  readonly navCollapsed = signal(false)
  // Per nav-group override — absent here just follows the route (see isMenuOpen).
  private readonly menuManualOverrides = signal<Record<string, boolean>>({})

  private readonly repositories = signal<RepositorySummary[]>([])
  private readonly users = signal<UserSummary[]>([])
  private readonly packages = signal<ReadableCatalogEntry[]>([])
  private readonly packageQueries = new Subject<string>()
  readonly searchQuery = signal('')
  /** A search load failed; typing again retries it. */
  readonly searchFailed = signal(false)

  // Whoever can reach /users (STAFF_ONLY_ACTIONS) can also search it.
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
      { label: 'Dépôts', icon: 'package', items: matchingRepositories },
    ]
    if (this.canSeeUsers()) {
      categories.push({ label: 'Utilisateurs', icon: 'user', items: matchingUsers })
    }
    const matchingPackages = this.packages().map((entry): SearchResult => ({
      kind: 'package',
      id: entry.repository.id,
      format: entry.kind,
      name: entry.name,
      label: packageLabel(entry),
    }))
    if (matchingPackages.length > 0) {
      categories.push({ label: 'Paquets et images', icon: 'package', items: matchingPackages })
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

  // Starts as '' — calling deepestRouteTitle() here would throw, too early in the route tree.
  private readonly routeTitle = toSignal(
    this.router.events.pipe(
      filter((event) => event instanceof NavigationEnd),
      map(() => this.deepestRouteTitle()),
    ),
    { initialValue: '' },
  )

  readonly navItems = computed<NavItem[]>(() => {
    const items: NavItem[] = [
      { action: 'explorer', icon: 'compass', text: 'Explorer', link: '/', exact: true },
      { action: 'repositories', icon: 'package', text: 'Dépôts', link: '/repositories' },
      { action: 'my-repository', icon: 'user', text: 'Mon dépôt', link: '/my-repository' },
      { action: 'users', icon: 'users', text: 'Utilisateurs', link: '/users' },
      {
        action: 'admin',
        icon: 'layout-dashboard',
        text: 'Administration',
        link: '/admin',
        children: [
          {
            action: 'organizations',
            icon: 'server',
            text: 'Organisations',
            link: '/admin/organizations',
          },
          { action: 'export', icon: 'download', text: 'Export', link: '/admin/export' },
          { action: 'health', icon: 'activity', text: 'Santé', link: '/admin/health' },
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
        text: 'Administration',
        link: `/admin/organizations/${organizationId}`,
      })
    }
    return filtered
  })

  constructor() {
    effect(() => this.pageTitle.title.set(this.routeTitle()))
    this.packageQueries
      .pipe(
        // a too-short query clears at once, a longer one waits for a pause in typing
        debounce((query) =>
          query.length < PACKAGE_SEARCH_MIN_LENGTH ? of(0) : timer(PACKAGE_SEARCH_DEBOUNCE_MS),
        ),
        distinctUntilChanged(),
        // switchMap drops the in-flight request, so a slow answer can never overwrite a newer one
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
        // Only a refused session ends it; a 5xx or network error keeps the token and offers a retry.
        if (error instanceof HttpErrorResponse && (error.status === 401 || error.status === 403)) {
          this.auth.logout()
          this.router.navigateByUrl('/login')
        } else {
          this.loadFailed.set(true)
        }
      },
    })
  }

  // Refresh only when a search starts, not on every keystroke — catches changes made elsewhere.
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
    return (route.snapshot.data['title'] as string | undefined) ?? ''
  }

  logout(): void {
    this.userMenuOpen.set(false)
    this.auth.logout()
    this.router.navigateByUrl('/login')
  }

  toggleUserMenu(): void {
    this.userMenuOpen.update((open) => !open)
  }

  // Opens automatically while on one of its own pages, until manually toggled.
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
