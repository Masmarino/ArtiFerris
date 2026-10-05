import { t } from '../shared/i18n/translator'
import { LanguageService } from '../shared/i18n/language.service'
import { TranslocoPipe } from '@jsverse/transloco'
import {
  ChangeDetectionStrategy,
  Component,
  OnInit,
  computed,
  effect,
  untracked,
  inject,
  signal,
} from '@angular/core'
import { HttpErrorResponse } from '@angular/common/http'
import { takeUntilDestroyed, toSignal } from '@angular/core/rxjs-interop'
import {
  ActivatedRouteSnapshot,
  NavigationEnd,
  Router,
  RouterLink,
  RouterLinkActive,
  RouterOutlet,
} from '@angular/router'
import { filter, map } from 'rxjs'
import { AppShell as GbtAppShell, AppShellNavGroup } from '@masmarino/gabarit/app-shell'
import { Breadcrumb } from '@masmarino/gabarit/breadcrumb'
import { CommandPaletteTrigger } from '@masmarino/gabarit/command-palette'
import { Button } from '@masmarino/gabarit/button'
import { Icon } from '@masmarino/gabarit/icon'
import { Menu, MenuItem } from '@masmarino/gabarit/menu'
import { Spinner } from '@masmarino/gabarit/spinner'
import { Toaster } from '@masmarino/gabarit/toaster'
import { AuthService } from '../auth/application/auth.service'
import { MeService } from './application/me.service'
import { VersionService } from './application/version.service'
import { NavItem } from './nav-item'
import { QuickSearch } from './quick-search/quick-search'
import { PageTitleService } from './page-title.service'
import { TrailStep, pageTrail } from './page-trail'
import { ConfirmHost } from '../shared/confirm-host/confirm-host'
import { ToastService } from '../shared/toast.service'

@Component({
  selector: 'app-shell',
  standalone: true,
  imports: [
    TranslocoPipe,
    RouterOutlet,
    RouterLink,
    RouterLinkActive,
    Icon,
    Breadcrumb,
    Menu,
    MenuItem,
    CommandPaletteTrigger,
    QuickSearch,
    Spinner,
    Toaster,
    Button,
    GbtAppShell,
    AppShellNavGroup,
    ConfirmHost,
  ],
  templateUrl: './app-shell.html',
  styleUrl: './app-shell.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class AppShell implements OnInit {
  private readonly auth = inject(AuthService)
  private readonly router = inject(Router)
  readonly me = inject(MeService)
  readonly version = inject(VersionService)
  readonly pageTitle = inject(PageTitleService)
  readonly toastService = inject(ToastService)
  private readonly languageService = inject(LanguageService)
  readonly viewGeneration = signal(0)

  readonly isLoading = signal(true)
  readonly loadFailed = signal(false)
  readonly navCollapsed = signal(false)
  /** What sits above the current page, from its route, translated: the bar's breadcrumb. */
  readonly trail = signal<{ label: string; link: string | null }[]>([])
  private readonly menuManualOverrides = signal<Record<string, boolean>>({})

  private readonly currentUrl = toSignal(
    this.router.events.pipe(
      filter((event) => event instanceof NavigationEnd),
      map(() => this.router.url),
    ),
    { initialValue: this.router.url },
  )

  // Starts empty: calling deepestRouteTitle() here is too early.
  // Starts from the route the shell opens on, then follows each navigation.
  private readonly routeTitle = toSignal(
    this.router.events.pipe(
      filter((event) => event instanceof NavigationEnd),
      map(() => this.deepestRouteTitle()),
    ),
    { initialValue: this.deepestRouteTitle() },
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
    ]
    const administration = this.administrationItems()
    if (administration.length > 0) {
      items.push({
        action: 'admin',
        icon: 'settings',
        text: t('nav.administration'),
        link: '/admin',
        children: administration,
      })
    }
    return items
  })

  /**
   * What the Administration group holds, as in FerrisGit: the dashboard first, the settings last. A super-admin reaches
   * the whole instance; an organization admin, its users and its own organization's settings.
   */
  private administrationItems(): NavItem[] {
    const users: NavItem = { action: 'users', icon: 'users', text: t('nav.users'), link: '/users' }
    if (this.me.isSuperAdmin()) {
      return [
        {
          action: 'dashboard',
          icon: 'layout-dashboard',
          text: t('nav.dashboard'),
          link: '/admin',
          exact: true,
        },
        users,
        {
          action: 'organizations',
          icon: 'server',
          text: t('nav.organizations'),
          link: '/admin/organizations',
        },
        { action: 'health', icon: 'activity', text: t('nav.health'), link: '/admin/health' },
        { action: 'export', icon: 'download', text: t('nav.export'), link: '/admin/export' },
      ]
    }
    const organizationId = this.me.organizationId()
    if (!this.me.isOrganizationAdmin() || !organizationId) {
      return []
    }
    return [
      users,
      {
        action: 'organization',
        icon: 'settings',
        text: t('nav.settings'),
        link: `/admin/organizations/${organizationId}`,
      },
    ]
  }

  constructor() {
    effect(() => this.pageTitle.title.set(this.routeTitle()))
    this.router.events
      .pipe(
        filter((event) => event instanceof NavigationEnd),
        takeUntilDestroyed(),
      )
      .subscribe(() => this.trail.set(this.translatedTrail()))
    // The route the shell opens on, before any navigation ends.
    this.trail.set(this.translatedTrail())
    let firstRun = true
    effect(() => {
      this.languageService.language()
      untracked(() => {
        if (firstRun) {
          firstRun = false
          return
        }
        this.viewGeneration.update((generation) => generation + 1)
        this.trail.set(this.translatedTrail())
        const title = this.deepestRouteTitle()
        if (title) {
          this.pageTitle.title.set(title)
        }
      })
    })
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

  private translatedTrail(): { label: string; link: string | null }[] {
    return pageTrail(this.router.routerState.snapshot.root).map((step: TrailStep) => ({
      label: t(step.labelKey),
      link: step.link ?? null,
    }))
  }

  /**
   * The router's snapshot, not the shell's ActivatedRoute chain: the shell is created while the
   * navigation to it is under way, before its child routes are activated and have a snapshot.
   */
  private deepestRouteTitle(): string {
    let route: ActivatedRouteSnapshot = this.router.routerState.snapshot.root
    while (route.firstChild) {
      route = route.firstChild
    }
    const titleKey = route.data['titleKey'] as string | undefined
    return titleKey ? t(titleKey) : ''
  }

  logout(): void {
    this.auth.logout()
    this.router.navigateByUrl('/login')
  }

  isMenuOpen(item: NavItem): boolean {
    const url = this.currentUrl()
    // Open on any of its pages: Utilisateurs lives at /users, outside /admin.
    const onChild = (item.children ?? []).some(
      (child) => url === child.link || url.startsWith(`${child.link}/`),
    )
    return this.menuManualOverrides()[item.action] ?? (url.startsWith(item.link) || onChild)
  }

  setMenuOpen(item: NavItem, open: boolean): void {
    this.menuManualOverrides.update((overrides) => ({ ...overrides, [item.action]: open }))
  }
}
