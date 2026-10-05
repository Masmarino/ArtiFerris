import { t } from '../../shared/i18n/translator'
import { TranslocoPipe } from '@jsverse/transloco'
import {
  ChangeDetectionStrategy,
  Component,
  computed,
  inject,
  input,
  output,
  signal,
} from '@angular/core'
import { takeUntilDestroyed, toSignal } from '@angular/core/rxjs-interop'
import { NavigationEnd, Params, Router } from '@angular/router'
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
import { CommandGroup, CommandItem, CommandPalette } from '@masmarino/gabarit/command-palette'
import { MeService } from '../application/me.service'
import { ReadableCatalogService } from '../application/readable-catalog.service'
import { ReadableCatalogEntry } from '../domain/readable-catalog.entity'
import { NavItem } from '../nav-item'
import { CATALOGS } from '../../public/catalog/domain/catalog.registry'
import { RepositoriesService } from '../../repositories/application/repositories.service'
import { RepositorySummary } from '../../repositories/domain/repository.entity'
import { UsersService } from '../../users/application/users.service'
import { UserSummary } from '../../users/domain/user.entity'
import { formatResultsAnnouncement } from '../../shared/format'
import { NEW_PARAM } from '../../shared/open-when-asked'
import { RecentRepositoriesService } from './recent-repositories.service'

/** Where a choice leads: a page, or signing out. */
export type QuickTarget = { link: string[]; queryParams?: Params } | { logout: true }

type QuickItem = CommandItem<QuickTarget>

const PACKAGE_SEARCH_MIN_LENGTH = 2
const PACKAGE_SEARCH_DEBOUNCE_MS = 250
const PACKAGE_SEARCH_LIMIT = 5

/** `/repositories/<id>` and anything under it, but not the list itself. */
const REPOSITORY_URL = /^\/repositories\/([^/?#]+)/

/**
 * The quick search, opened with ⌘K (Ctrl K elsewhere), `/` or the header's `gbt-command-palette-trigger` (give it this
 * component): the repositories opened last, the rail's pages — the shell's own entries, so the administration only
 * shows to whom the rail shows it — and the creation actions, filtered as one types, then the repositories, users and
 * packages that match. Placed outside the header, whose dark theme it would otherwise take on.
 */
@Component({
  selector: 'app-quick-search',
  imports: [CommandPalette, TranslocoPipe],
  templateUrl: './quick-search.html',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class QuickSearch {
  private readonly router = inject(Router)
  private readonly me = inject(MeService)
  private readonly repositoriesService = inject(RepositoriesService)
  private readonly usersService = inject(UsersService)
  private readonly readableCatalog = inject(ReadableCatalogService)
  private readonly recent = inject(RecentRepositoriesService)

  /** The rail's entries, as the shell computed them for this account. */
  readonly navItems = input.required<NavItem[]>()
  /** Signing out belongs to the shell, which also offers it from the account menu. */
  readonly logout = output<void>()

  protected readonly shortcuts = ['mod+k', '/']
  protected readonly open = signal(false)
  protected readonly query = signal('')
  protected readonly searching = signal(false)
  /** The repositories or users could not be loaded on opening. */
  private readonly listsFailed = signal(false)
  /** The last package search failed; the next one tries again. */
  private readonly packagesFailed = signal(false)
  protected readonly searchFailed = computed(() => this.listsFailed() || this.packagesFailed())
  private readonly repositories = signal<RepositorySummary[]>([])
  private readonly users = signal<UserSummary[]>([])
  private readonly packages = signal<ReadableCatalogEntry[]>([])
  private readonly packageQueries = new Subject<string>()

  /** The repository on screen, if any: already where one is, so not offered back. */
  private readonly currentRepositoryId = toSignal(
    this.router.events.pipe(
      filter((event): event is NavigationEnd => event instanceof NavigationEnd),
      map((event) => repositoryIdIn(event.urlAfterRedirects)),
    ),
    { initialValue: repositoryIdIn(this.router.url) },
  )

  // Whoever can reach /users can also search it.
  private readonly canSeeUsers = computed(
    () => this.me.isSuperAdmin() || this.me.isOrganizationAdmin(),
  )

  protected readonly resultsAnnouncement = formatResultsAnnouncement

  protected readonly groups = computed<CommandGroup<QuickTarget>[]>(() => {
    const query = this.query().trim()
    const groups: CommandGroup<QuickTarget>[] = []
    const recents = this.recentItems()
    if (recents.length > 0) {
      groups.push({ label: t('shell.search.recent'), items: recents })
    }
    groups.push({ label: t('shell.search.goTo'), items: this.pageItems() })
    groups.push({ label: t('shell.search.actions'), items: this.actionItems() })
    // Every repository and user would drown the pages: they come once something is typed, filtered as the pages are.
    if (query) {
      groups.push({ label: t('nav.repositories'), items: this.repositories().map(repositoryItem) })
      if (this.canSeeUsers()) {
        groups.push({ label: t('nav.users'), items: this.users().map(userItem) })
      }
      // Found by the server, so already matching: the palette mustn't filter them again on its own terms.
      groups.push({
        label: t('shell.search.packagesAndImages'),
        filter: false,
        items: this.packages().map(packageItem),
      })
    }
    return groups
  })

  constructor() {
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
                  this.packagesFailed.set(true)
                  return of([])
                }),
              ),
        ),
        takeUntilDestroyed(),
      )
      .subscribe((entries) => {
        this.packages.set(entries)
        this.searching.set(false)
      })

    // Every repository opened joins the recent ones, whichever way it was reached.
    this.router.events
      .pipe(
        filter((event): event is NavigationEnd => event instanceof NavigationEnd),
        map((event) => repositoryIdIn(event.urlAfterRedirects)),
        filter((id): id is string => id !== null),
        takeUntilDestroyed(),
      )
      .subscribe((id) => this.recent.remember(this.me.username(), id))
  }

  /** Opens the palette, for the header's trigger. */
  show(): void {
    this.onOpenChange(true)
  }

  protected onOpenChange(open: boolean): void {
    this.open.set(open)
    if (open) {
      // Fresh lists on every opening, to catch changes made in another tab.
      this.refreshLists()
    }
  }

  protected onQueryChange(query: string): void {
    this.query.set(query)
    const trimmed = query.trim()
    this.packagesFailed.set(false)
    this.searching.set(trimmed.length >= PACKAGE_SEARCH_MIN_LENGTH)
    this.packageQueries.next(trimmed)
  }

  protected onSelect(item: QuickItem): void {
    const target = item.data
    if (!target) {
      return
    }
    if ('logout' in target) {
      this.logout.emit()
      return
    }
    void this.router.navigate(target.link, { queryParams: target.queryParams })
  }

  private refreshLists(): void {
    this.listsFailed.set(false)
    const failed = () => this.listsFailed.set(true)
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

  private recentItems(): QuickItem[] {
    const byId = new Map(this.repositories().map((repository) => [repository.id, repository]))
    // One no longer listed is gone or out of reach.
    const here = this.currentRepositoryId()
    return this.recent
      .list(this.me.username())
      .filter((id) => id !== here)
      .map((id) => byId.get(id))
      .filter((repository): repository is RepositorySummary => repository !== undefined)
      .map((repository) => ({
        ...repositoryItem(repository),
        id: `recent:${repository.id}`,
        icon: 'clock',
      }))
  }

  private pageItems(): QuickItem[] {
    const pages: QuickItem[] = []
    for (const item of this.navItems()) {
      if (item.children) {
        for (const child of item.children) {
          pages.push(pageItem(child, item.text))
        }
      } else {
        pages.push(pageItem(item))
      }
    }
    pages.push(
      { id: 'page:account', label: t('nav.account'), icon: 'user', data: { link: ['/account'] } },
      { id: 'page:docs', label: t('nav.docs'), icon: 'book-open', data: { link: ['/docs'] } },
    )
    return pages
  }

  private actionItems(): QuickItem[] {
    const actions: QuickItem[] = [
      {
        id: 'action:new-repository',
        label: t('repositories.list.newRepository'),
        icon: 'plus',
        data: { link: ['/repositories'], queryParams: { [NEW_PARAM]: 'repository' } },
      },
      {
        id: 'action:new-project',
        label: t('repositories.list.newProject'),
        icon: 'plus',
        data: { link: ['/my-repository'], queryParams: { [NEW_PARAM]: 'project' } },
      },
    ]
    // As on the users page: only a super-admin invites.
    if (this.me.isSuperAdmin()) {
      actions.push({
        id: 'action:invite',
        label: t('users.list.invite'),
        icon: 'user-plus',
        data: { link: ['/users'], queryParams: { [NEW_PARAM]: 'user' } },
      })
    }
    actions.push({
      id: 'action:logout',
      label: t('shell.logout'),
      icon: 'log-out',
      data: { logout: true },
    })
    return actions
  }
}

function repositoryIdIn(url: string): string | null {
  return REPOSITORY_URL.exec(url)?.[1] ?? null
}

function pageItem(item: NavItem, group?: string): QuickItem {
  return {
    id: `page:${item.link}`,
    label: item.text,
    description: group,
    icon: item.icon,
    data: { link: [item.link] },
  }
}

function repositoryItem(repository: RepositorySummary): QuickItem {
  return {
    id: `repository:${repository.id}`,
    label: repository.name,
    description: repository.owner_is_personal ? `@${repository.owner_name}` : repository.owner_name,
    icon: 'package',
    data: { link: ['/repositories', repository.id] },
  }
}

function userItem(user: UserSummary): QuickItem {
  return {
    id: `user:${user.id}`,
    label: user.username,
    icon: 'user',
    data: { link: ['/users', user.id] },
  }
}

function packageItem(entry: ReadableCatalogEntry): QuickItem {
  const format = CATALOGS.find((catalog) => catalog.format === entry.kind)?.label ?? entry.kind
  return {
    id: `package:${entry.repository.id}:${entry.kind}:${entry.name}`,
    label: entry.name,
    description:
      entry.repository.repo_type === 'proxy'
        ? `${format}, ${entry.repository.name} (${t('shell.search.proxyCache')})`
        : `${format}, ${entry.repository.name}`,
    icon: 'package',
    data: { link: ['/repositories', entry.repository.id, 'packages', entry.kind, entry.name] },
  }
}
