import { activeLocale, t } from '../../shared/i18n/translator'
import { TranslocoPipe } from '@jsverse/transloco'
import {
  ChangeDetectionStrategy,
  Component,
  ElementRef,
  Injector,
  OnInit,
  afterNextRender,
  computed,
  inject,
  signal,
  viewChild,
} from '@angular/core'
import { HttpErrorResponse } from '@angular/common/http'
import { Router, RouterLink } from '@angular/router'
import { FormsModule } from '@angular/forms'
import { forkJoin } from 'rxjs'
import { Badge } from '@masmarino/gabarit/badge'
import { Button } from '@masmarino/gabarit/button'
import { ListCard, type ListCardState } from '@masmarino/gabarit/list-card'
import { ListRow } from '@masmarino/gabarit/list-row'
import {
  ListToolbar,
  createListToolbarState,
  type ListToolbarSortOption,
} from '@masmarino/gabarit/list-toolbar'
import { Menu, MenuItem } from '@masmarino/gabarit/menu'
import { PageLayout } from '@masmarino/gabarit/page-layout'
import { Panel } from '@masmarino/gabarit/panel'
import { SegmentedControl, type SegmentedControlOption } from '@masmarino/gabarit/segmented-control'
import { Select, type SelectOption } from '@masmarino/gabarit/select'
import { Spinner } from '@masmarino/gabarit/spinner'
import { UserChip } from '@masmarino/gabarit/user-chip'
import { CreateUserModal } from '../create-user-modal/create-user-modal'
import { LinkMailFailed, MailedLinkKind } from '../../shared/link-mail-failed/link-mail-failed'
import { InvitationMail, PasswordResetMail } from '../../shared/invitation-mail'
import { UsersService } from '../application/users.service'
import { UserSummary, displayName } from '../domain/user.entity'
import { OrganizationsService } from '../../admin/application/organizations.service'
import { OrganizationSummary } from '../../admin/domain/organization.entity'
import { MeService } from '../../shell/application/me.service'
import { AuthService } from '../../auth/application/auth.service'
import { PageHeading } from '../../shared/page-heading/page-heading'
import { ConfirmService } from '../../shared/confirm.service'
import { ToastService } from '../../shared/toast.service'
import { RowDate, rowDate } from '../../shared/row-date'
import {
  Presentation,
  SUPER_ADMIN,
  accountState,
  adminAction,
  mfaPresentation,
  resetMfaFailure,
  resetMfaMessage,
  resetPasswordFailure,
} from './user-presentation'
import { openWhenAsked } from '../../shared/open-when-asked'

/** Not a real organization id: selects the unfiltered view across organizations. */
const ALL_ORGANIZATIONS = 'ALL'

type UsersFilter = 'all' | 'pending'
type SortKey = 'createdAt' | 'username'
type LoadState = 'loading' | 'loaded' | 'failed'

interface UserRow {
  user: UserSummary
  name: string
  isSelf: boolean
  state: Presentation
  mfa: Presentation | null
  created: RowDate
  expiry: RowDate | null
  organization: string | null
  canResend: boolean
  canResetMfa: boolean
  canResetPassword: boolean
  adminAction: { label: string; icon: string } | null
  menuLabel: string
  /** Null for the signed-in administrator's own account, as in FerrisGit. */
  link: string[] | null
  busy: string | null
}

/**
 * The organization's accounts, laid out like FerrisGit's administration: a header that sums them
 * up, search and sort, all accounts or the pending invitations, rows with their state and second
 * factor, actions in each row's menu, and what they do explained on the side. A super-admin also
 * picks the organization, invites, and grants or removes super-administrator rights.
 */
@Component({
  selector: 'app-users-list',
  standalone: true,
  imports: [
    PageHeading,
    TranslocoPipe,
    RouterLink,
    FormsModule,
    Badge,
    Button,
    ListCard,
    ListRow,
    ListToolbar,
    Menu,
    MenuItem,
    PageLayout,
    Panel,
    SegmentedControl,
    Select,
    Spinner,
    UserChip,
    CreateUserModal,
    LinkMailFailed,
  ],
  templateUrl: './users-list.html',
  styleUrl: './users-list.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class UsersList implements OnInit {
  private readonly usersService = inject(UsersService)
  private readonly organizationsService = inject(OrganizationsService)
  private readonly me = inject(MeService)
  private readonly confirm = inject(ConfirmService)
  private readonly toast = inject(ToastService)
  private readonly host = inject<ElementRef<HTMLElement>>(ElementRef)
  private readonly injector = inject(Injector)
  private readonly auth = inject(AuthService)
  private readonly router = inject(Router)

  // The organization filter, "Inviter" and the super-admin rights are for super-admins: an org
  // admin's /api/users is already scoped to their organization.
  readonly isSuperAdmin = computed(() => this.me.isSuperAdmin())

  readonly users = signal<UserSummary[]>([])
  readonly organizations = signal<OrganizationSummary[]>([])
  readonly selectedOrganizationId = signal<string>(ALL_ORGANIZATIONS)
  readonly showCreateModal = signal(false)
  readonly loadState = signal<LoadState>('loading')

  readonly organizationOptions = computed<SelectOption<string>[]>(() => [
    { value: ALL_ORGANIZATIONS, label: t('users.list.allOrganizations') },
    ...this.organizations().map((o) => ({ value: o.id, label: o.display_name })),
  ])

  private readonly organizationNamesById = computed(
    () => new Map(this.organizations().map((o) => [o.id, o.display_name])),
  )

  private readonly inOrganization = computed(() => {
    const organizationId = this.selectedOrganizationId()
    return organizationId === ALL_ORGANIZATIONS
      ? this.users()
      : this.users().filter((u) => u.organization_id === organizationId)
  })

  protected readonly cardState = computed<ListCardState>(() =>
    this.loadState() === 'loading'
      ? 'loading'
      : this.loadState() === 'failed'
        ? 'failed'
        : this.users().length === 0
          ? 'empty'
          : 'ready',
  )

  protected readonly filter = signal<UsersFilter>('all')
  private readonly pendingUsers = computed(() =>
    this.inOrganization().filter((user) => user.invitation_pending),
  )
  private readonly filtered = computed(() =>
    this.filter() === 'pending' ? this.pendingUsers() : this.inOrganization(),
  )

  protected readonly sortOptions: ListToolbarSortOption<SortKey>[] = [
    { value: 'createdAt', label: t('users.list.sortCreated') },
    { value: 'username', label: t('users.list.sortUsername') },
  ]
  protected readonly searchSort = createListToolbarState<SortKey>({
    sortOptions: this.sortOptions,
    defaultSort: 'createdAt',
  })
  protected readonly search = this.searchSort.search
  protected readonly sortValue = this.searchSort.sortValue
  protected readonly direction = this.searchSort.direction
  private readonly filteredUsers = this.searchSort.filtered(() => this.filtered(), {
    text: (user) => [displayName(user), user.email ?? ''],
    sortBy: { username: (user) => displayName(user), createdAt: (user) => user.created_at },
    locale: activeLocale(),
  })

  /** Shown from the first paint so the layout doesn't shift when the list arrives. */
  protected readonly toolbarShown = computed(
    () => this.loadState() !== 'loaded' || this.inOrganization().length > 0,
  )
  protected readonly toolbarActive = computed(() => this.loadState() === 'loaded')

  protected readonly filterOptions = computed<SegmentedControlOption<UsersFilter>[]>(() => [
    { value: 'all', label: t('users.list.tabAll', { count: this.inOrganization().length }) },
    {
      value: 'pending',
      label: t('users.list.tabPending', { count: this.pendingUsers().length }),
    },
  ])

  protected readonly summary = computed(() => {
    const total = this.inOrganization().length
    if (this.loadState() !== 'loaded' || total === 0) {
      return null
    }
    const accounts = t(total === 1 ? 'users.list.accounts_one' : 'users.list.accounts_other', {
      count: total,
    })
    const pending = this.pendingUsers().length
    return pending === 0
      ? accounts
      : t('users.list.summaryWithPending', {
          accounts,
          pending: t(
            pending === 1 ? 'users.list.pendingCount_one' : 'users.list.pendingCount_other',
            { count: pending },
          ),
        })
  })

  private readonly busyIds = signal<ReadonlyMap<string, string>>(new Map())
  /** A resent invitation whose mail did not go out: its new link, shown until dismissed. */
  protected readonly mailFailure = signal<{
    name: string
    mail: InvitationMail | PasswordResetMail
    kind: MailedLinkKind
  } | null>(null)
  private readonly mailFailedAlert = viewChild(LinkMailFailed)

  protected readonly rows = computed<UserRow[]>(() => {
    const now = new Date()
    const busy = this.busyIds()
    const self = this.me.username()
    const showOrganization =
      this.isSuperAdmin() && this.selectedOrganizationId() === ALL_ORGANIZATIONS
    return this.filteredUsers().map((user) =>
      this.toRow(user, user.username === self, busy.get(user.id) ?? null, now, showOrganization),
    )
  })

  private toRow(
    user: UserSummary,
    isSelf: boolean,
    busy: string | null,
    now: Date,
    showOrganization: boolean,
  ): UserRow {
    const name = displayName(user)
    const { state, expiry } = accountState(user, now)
    // An organization admin can't act on a super-admin's account: that is a global privilege.
    const canAct = this.isSuperAdmin() || !user.is_super_admin
    return {
      user,
      name,
      isSelf,
      state,
      mfa: mfaPresentation(user),
      created: rowDate(user.created_at, now, 'users.row.createdAgo', 'users.row.createdOn'),
      expiry,
      organization: showOrganization
        ? (this.organizationNamesById().get(user.organization_id) ?? null)
        : null,
      canResend: user.invitation_pending && canAct,
      canResetMfa: !user.invitation_pending && user.mfa_enabled && canAct,
      // One's own password is changed from "Mon compte": a lost mail would lock an administrator out.
      canResetPassword: !user.invitation_pending && canAct && !isSelf,
      adminAction:
        this.isSuperAdmin() && !user.invitation_pending ? adminAction(user.is_super_admin) : null,
      menuLabel: t('users.list.menuLabel', { name }),
      link: isSelf ? null : ['/users', user.id],
      busy,
    }
  }

  protected readonly superAdmin = SUPER_ADMIN
  protected readonly hasNoResults = computed(() => this.rows().length === 0)
  protected readonly noResultsText = computed(() => {
    if (this.inOrganization().length === 0) {
      return t('users.list.noneInOrganization')
    }
    const searching = this.search().trim() !== ''
    if (this.filter() === 'pending') {
      return t(searching ? 'users.list.noPendingMatch' : 'users.list.noPending')
    }
    return t('users.list.noMatch')
  })

  // Set once: a later reload() must not reset the viewer's pick.
  private hasAppliedDefaultOrganizationFilter = false

  constructor() {
    // Newest first: a fresh invitation is what an administrator comes back to.
    this.direction.set('desc')
    // The quick search's "Inviter un utilisateur", for super-admins as the button is.
    openWhenAsked('user', () => this.showCreateModal.set(this.isSuperAdmin()))
  }

  ngOnInit(): void {
    this.reload()
  }

  reload(): void {
    if (!this.isSuperAdmin()) {
      // /api/organizations is super-admin only: no forkJoin here.
      this.usersService.list({ forceRefresh: true }).subscribe({
        next: (users) => this.loaded(users),
        error: () => this.loadFailed(),
      })
      return
    }
    forkJoin({
      users: this.usersService.list({ forceRefresh: true }),
      organizations: this.organizationsService.list(),
    }).subscribe({
      next: ({ users, organizations }) => {
        this.organizations.set(organizations)
        if (!this.hasAppliedDefaultOrganizationFilter) {
          // Defaults to the public organization, not all of them.
          const publicOrganization = organizations.find((o) => o.is_public)
          this.selectedOrganizationId.set(publicOrganization?.id ?? ALL_ORGANIZATIONS)
          this.hasAppliedDefaultOrganizationFilter = true
        }
        this.loaded(users)
      },
      error: () => this.loadFailed(),
    })
  }

  private loaded(users: UserSummary[]): void {
    this.users.set(users)
    this.loadState.set('loaded')
  }

  /** A failed reload keeps the list and says so; a failed first load shows the card's alert. */
  private loadFailed(): void {
    if (this.loadState() !== 'loaded') {
      this.loadState.set('failed')
      return
    }
    this.toast.error(t('users.list.errors.loadFailed'))
  }

  protected retryLoad(): void {
    this.loadState.set('loading')
    this.reload()
  }

  resend(row: UserRow): void {
    const user = row.user
    if (this.busyIds().has(user.id)) return
    this.setBusy(user.id, t('users.list.sending'))
    this.usersService.resendInvitation(user.id).subscribe({
      next: (mail) => {
        this.setBusy(user.id, null)
        this.reload()
        if (mail.email_sent) {
          this.mailFailure.set(null)
          this.toast.success(t('users.list.resent', { email: user.email ?? row.name }))
          this.focusRowAction(user.id)
          return
        }
        // The old link no longer works: the new one is the only way in, so it goes where it is read first.
        this.mailFailure.set({ name: row.name, mail, kind: 'invitation' })
        afterNextRender(() => this.mailFailedAlert()?.focus(), { injector: this.injector })
      },
      error: () => {
        this.setBusy(user.id, null)
        this.toast.error(t('users.detail.errors.resendFailed'))
        this.focusRowAction(user.id)
      },
    })
  }

  /** The old password stops working at once; the link goes by mail, or comes back when the mail cannot go out. */
  async resetPassword(row: UserRow): Promise<void> {
    const user = row.user
    if (this.busyIds().has(user.id)) return
    const confirmed = await this.confirm.ask({
      heading: t('users.passwordReset.heading'),
      message: t('users.passwordReset.message', { username: row.name }),
      confirmLabel: t('users.passwordReset.confirm'),
      danger: true,
    })
    if (!confirmed) {
      this.focusRowAction(user.id)
      return
    }
    this.setBusy(user.id, t('users.passwordReset.resetting'))
    this.usersService.resetPassword(user.id).subscribe({
      next: (mail) => {
        this.setBusy(user.id, null)
        if (mail.email_sent) {
          this.mailFailure.set(null)
          this.toast.success(t('users.passwordReset.sent', { username: row.name }))
          this.focusRowAction(user.id)
          return
        }
        this.mailFailure.set({ name: row.name, mail, kind: 'password-reset' })
        afterNextRender(() => this.mailFailedAlert()?.focus(), { injector: this.injector })
      },
      error: (error: unknown) => {
        this.setBusy(user.id, null)
        this.toast.error(resetPasswordFailure(error))
        this.focusRowAction(user.id)
      },
    })
  }

  /** For someone who lost every factor. One's own account signs out at once, like everyone else's sessions. */
  async resetMfa(row: UserRow): Promise<void> {
    const user = row.user
    if (this.busyIds().has(user.id)) return
    const confirmed = await this.confirm.ask({
      heading: t('users.mfaReset.heading'),
      message: resetMfaMessage(row.name, row.isSelf),
      confirmLabel: t('users.mfaReset.confirm'),
      danger: true,
    })
    if (!confirmed) {
      this.focusRowAction(user.id)
      return
    }
    this.setBusy(user.id, t('users.mfaReset.resetting'))
    this.usersService.resetMfa(user.id).subscribe({
      next: () => {
        this.setBusy(user.id, null)
        if (row.isSelf) {
          this.auth.logout()
          void this.router.navigate(['/login'])
          return
        }
        this.users.update((list) =>
          list.map((item) => (item.id === user.id ? { ...item, mfa_enabled: false } : item)),
        )
        this.toast.success(t('users.mfaReset.done'))
        this.focusRowAction(user.id)
      },
      error: (error: unknown) => {
        this.setBusy(user.id, null)
        this.toast.error(resetMfaFailure(error))
        this.focusRowAction(user.id)
      },
    })
  }

  /** Granting is immediate; removing asks first, since it takes the administration away. */
  async toggleSuperAdmin(row: UserRow): Promise<void> {
    const user = row.user
    if (this.busyIds().has(user.id)) return
    const granting = !user.is_super_admin
    if (!granting) {
      const confirmed = await this.confirm.ask({
        heading: t('users.detail.demoteHeading'),
        message: t(row.isSelf ? 'users.list.demoteSelfMessage' : 'users.list.demoteMessage', {
          username: row.name,
        }),
        confirmLabel: t('users.detail.demoteAction'),
        danger: true,
      })
      if (!confirmed) {
        this.focusRowAction(user.id)
        return
      }
    }
    this.setBusy(user.id, t('users.list.saving'))
    this.usersService.setSuperAdmin(user.id, granting).subscribe({
      next: () => {
        this.setBusy(user.id, null)
        this.users.update((list) =>
          list.map((item) => (item.id === user.id ? { ...item, is_super_admin: granting } : item)),
        )
        this.toast.success(
          t(granting ? 'users.detail.promoted' : 'users.detail.demoted', { username: row.name }),
        )
        this.focusRowAction(user.id)
      },
      error: (error: unknown) => {
        this.setBusy(user.id, null)
        const lastSuperAdmin = error instanceof HttpErrorResponse && error.status === 409
        this.toast.error(
          t(
            lastSuperAdmin
              ? 'users.detail.errors.lastSuperAdmin'
              : 'users.detail.errors.superAdminStatus',
          ),
        )
        this.focusRowAction(user.id)
      },
    })
  }

  /** Puts focus back after render: the menu closed or became the spinner, and took the focus with it. */
  private focusRowAction(userId: string): void {
    afterNextRender(
      () => {
        const row = Array.from(
          this.host.nativeElement.querySelectorAll<HTMLElement>('li[data-user-id]'),
        ).find((item) => item.dataset['userId'] === userId)
        ;(row?.querySelector<HTMLElement>('button[aria-haspopup="menu"]') ?? row)?.focus()
      },
      { injector: this.injector },
    )
  }

  private setBusy(id: string, label: string | null): void {
    this.busyIds.update((ids) => {
      const next = new Map(ids)
      if (label) {
        next.set(id, label)
      } else {
        next.delete(id)
      }
      return next
    })
  }
}
