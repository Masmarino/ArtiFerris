import { t } from '../../shared/i18n/translator'
import { TranslocoPipe } from '@jsverse/transloco'
import {
  ChangeDetectionStrategy,
  Component,
  Injector,
  afterNextRender,
  computed,
  effect,
  inject,
  signal,
  viewChild,
} from '@angular/core'
import { toSignal } from '@angular/core/rxjs-interop'
import { ActivatedRoute, Router, RouterLink } from '@angular/router'
import { HttpErrorResponse } from '@angular/common/http'
import { FormsModule } from '@angular/forms'
import { forkJoin, map } from 'rxjs'
import { Badge } from '@masmarino/gabarit/badge'
import { Button } from '@masmarino/gabarit/button'
import { Card } from '@masmarino/gabarit/card'
import { DescriptionList } from '@masmarino/gabarit/description-list'
import { EmptyState } from '@masmarino/gabarit/empty-state'
import { ListCard, type ListCardState } from '@masmarino/gabarit/list-card'
import { ListRow } from '@masmarino/gabarit/list-row'
import { Menu, MenuItem } from '@masmarino/gabarit/menu'
import { PageLayout } from '@masmarino/gabarit/page-layout'
import { Panel } from '@masmarino/gabarit/panel'
import { Select } from '@masmarino/gabarit/select'
import { Skeleton } from '@masmarino/gabarit/skeleton'
import { Spinner } from '@masmarino/gabarit/spinner'
import { PermissionRoleEditor } from '../../repositories/permission-role-editor/permission-role-editor'
import { PermissionsService } from '../../repositories/application/permissions.service'
import {
  ROLE_OPTIONS,
  Role,
  UserPermissionEntry,
} from '../../repositories/domain/permission.entity'
import { RepositoriesService } from '../../repositories/application/repositories.service'
import { RepositorySummary } from '../../repositories/domain/repository.entity'
import { PageTitleService } from '../../shell/page-title.service'
import { UsersService } from '../application/users.service'
import { UserSummary, displayName } from '../domain/user.entity'
import { formatSelectedCount } from '../../shared/format'
import { MeService } from '../../shell/application/me.service'
import { ToastService } from '../../shared/toast.service'
import { ConfirmService } from '../../shared/confirm.service'
import { PageHeading } from '../../shared/page-heading/page-heading'
import { LinkMailFailed, MailedLinkKind } from '../../shared/link-mail-failed/link-mail-failed'
import { InvitationMail, PasswordResetMail } from '../../shared/invitation-mail'
import { rowDate } from '../../shared/row-date'
import {
  SUPER_ADMIN,
  accountState,
  adminAction,
  mfaPresentation,
  resetMfaFailure,
  resetMfaMessage,
  resetPasswordFailure,
} from '../users-list/user-presentation'

type View = 'loading' | 'ready' | 'not-found' | 'self' | 'failed'

/**
 * One account, laid out like FerrisGit's administration: its name with its state and second factor,
 * its address and dates, the actions in a menu and the deletion beside it; then the repositories it
 * can reach and how to grant more, with what roles and deletion mean on the side.
 */
@Component({
  selector: 'app-user-detail',
  standalone: true,
  imports: [
    PageHeading,
    TranslocoPipe,
    RouterLink,
    FormsModule,
    Badge,
    Button,
    Card,
    DescriptionList,
    EmptyState,
    ListCard,
    ListRow,
    Menu,
    MenuItem,
    PageLayout,
    Panel,
    Select,
    Skeleton,
    Spinner,
    PermissionRoleEditor,
    LinkMailFailed,
  ],
  templateUrl: './user-detail.html',
  styleUrl: './user-detail.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class UserDetail {
  private readonly route = inject(ActivatedRoute)
  private readonly router = inject(Router)
  private readonly usersService = inject(UsersService)
  private readonly toastService = inject(ToastService)
  private readonly confirmService = inject(ConfirmService)
  private readonly permissionsService = inject(PermissionsService)
  private readonly repositoriesService = inject(RepositoriesService)
  private readonly pageTitle = inject(PageTitleService)
  private readonly me = inject(MeService)
  private readonly injector = inject(Injector)

  // Reactive, not route.snapshot: Angular reuses this component across navigations.
  private readonly routeUserId = toSignal(
    this.route.paramMap.pipe(map((params) => params.get('id')!)),
    {
      requireSync: true,
    },
  )
  private userId!: string

  readonly user = signal<UserSummary | null>(null)
  readonly loadError = signal(false)
  private readonly notFound = signal(false)
  readonly selectedCountLabel = formatSelectedCount
  // An invited account has no username until it is activated: its address names it.
  readonly username = computed(() => {
    const user = this.user()
    return user ? displayName(user) : ''
  })

  // Super-admin status is not an organization-scoped right.
  readonly isSuperAdminViewer = computed(() => this.me.isSuperAdmin())
  // Hidden for a super-admin target: the backend 403s an org admin there.
  readonly canManageTarget = computed(
    () => this.isSuperAdminViewer() || !this.user()?.is_super_admin,
  )

  /** One's own account is managed from "Mon compte", as in FerrisGit. */
  private readonly isSelf = computed(() => {
    const user = this.user()
    return !!user && !user.invitation_pending && user.username === this.me.username()
  })

  protected readonly view = computed<View>(() => {
    if (this.notFound()) return 'not-found'
    if (this.loadError()) return 'failed'
    if (!this.user()) return 'loading'
    return this.isSelf() ? 'self' : 'ready'
  })

  protected readonly superAdmin = SUPER_ADMIN
  protected readonly header = computed(() => {
    const user = this.user()
    if (!user) return null
    const now = new Date()
    const { state, expiry } = accountState(user, now)
    return {
      state,
      expiry,
      mfa: mfaPresentation(user),
      created: rowDate(user.created_at, now, 'users.row.createdAgo', 'users.row.createdOn'),
    }
  })

  /** What the "Actions" menu offers; null when it would be empty. */
  readonly actions = computed(() => {
    const user = this.user()
    if (!user) return null
    const canResend = user.invitation_pending && this.canManageTarget()
    const admin =
      this.isSuperAdminViewer() && !user.invitation_pending
        ? adminAction(user.is_super_admin)
        : null
    const canResetMfa = !user.invitation_pending && user.mfa_enabled && this.canManageTarget()
    const canResetPassword = !user.invitation_pending && this.canManageTarget()
    return canResend || canResetPassword || canResetMfa || admin
      ? { canResend, canResetPassword, canResetMfa, admin }
      : null
  })

  readonly permissions = signal<UserPermissionEntry[]>([])
  private readonly permissionsLoaded = signal(false)
  readonly editingPermission = signal<UserPermissionEntry | null>(null)
  readonly settingSuperAdmin = signal(false)
  readonly resendingInvitation = signal(false)
  readonly resettingMfa = signal(false)
  readonly resettingPassword = signal(false)
  readonly savingRole = signal(false)
  /** A resent invitation whose mail did not go out: its new link, shown until dismissed. */
  protected readonly mailFailure = signal<{
    mail: InvitationMail | PasswordResetMail
    kind: MailedLinkKind
  } | null>(null)
  private readonly mailFailedAlert = viewChild(LinkMailFailed)

  /** The spinner that stands in for the menu while one of its actions runs. */
  protected readonly busy = computed(() =>
    this.resendingInvitation()
      ? t('users.list.sending')
      : this.resettingPassword()
        ? t('users.passwordReset.resetting')
        : this.resettingMfa()
          ? t('users.mfaReset.resetting')
          : this.settingSuperAdmin()
            ? t('users.list.saving')
            : null,
  )

  readonly repositories = signal<RepositorySummary[]>([])
  // A repository already reachable is changed from its row, not granted again.
  readonly repositoryOptions = computed(() => {
    const granted = new Set(this.permissions().map((entry) => entry.repository_id))
    return this.repositories()
      .filter((repo) => !granted.has(repo.id))
      .map((repo) => ({ value: repo.id, label: repo.name }))
  })
  readonly grantRepositoryIds = signal<string[]>([])
  readonly grantRole = signal<Role>('read')
  readonly roleOptions = ROLE_OPTIONS
  readonly granting = signal(false)

  protected readonly permissionsCard = computed<ListCardState>(() =>
    !this.permissionsLoaded() ? 'loading' : this.permissions().length === 0 ? 'empty' : 'ready',
  )
  protected readonly permissionsSummary = computed(() => {
    const count = this.permissions().length
    if (!this.permissionsLoaded() || count === 0) return null
    return t(
      count === 1 ? 'users.detail.repositoryCount_one' : 'users.detail.repositoryCount_other',
      { count },
    )
  })
  protected readonly permissionFacts = computed(() => {
    const permissions = this.permissions()
    const count = (role: Role) => String(permissions.filter((entry) => entry.role === role).length)
    return [
      { term: t('users.detail.facts.repositories'), value: String(permissions.length) },
      ...ROLE_OPTIONS.map((option) => ({ term: option.label, value: count(option.value) })),
    ]
  })
  protected readonly skeletonLines = ['60%', '45%', '70%', '40%']

  constructor() {
    effect(() => this.pageTitle.title.set(this.username()))
    effect(() => {
      this.userId = this.routeUserId()
      this.resetForNewUser()
      this.reload()
      this.repositoriesService
        .list()
        .subscribe((repositories) => this.repositories.set(repositories))
    })
  }

  // Reused across navigations: drop the previous data.
  private resetForNewUser(): void {
    this.user.set(null)
    this.notFound.set(false)
    this.loadError.set(false)
    this.permissions.set([])
    this.permissionsLoaded.set(false)
    this.editingPermission.set(null)
    this.grantRepositoryIds.set([])
    this.granting.set(false)
    this.savingRole.set(false)
    this.settingSuperAdmin.set(false)
    this.resendingInvitation.set(false)
    this.resettingMfa.set(false)
    this.resettingPassword.set(false)
    this.mailFailure.set(null)
  }

  private reload(): void {
    const requestedId = this.userId
    this.usersService.get(requestedId).subscribe({
      next: (user) => {
        if (requestedId === this.userId) {
          this.user.set(user)
        }
      },
      error: (error: unknown) => {
        if (requestedId !== this.userId) return
        if (error instanceof HttpErrorResponse && error.status === 404) {
          this.user.set(null)
          this.notFound.set(true)
        } else {
          this.loadError.set(true)
        }
      },
    })
    this.permissionsService.listForUser(requestedId).subscribe({
      next: (permissions) => {
        if (requestedId === this.userId) {
          this.permissions.set(permissions)
          this.permissionsLoaded.set(true)
        }
      },
      error: () => {
        if (requestedId === this.userId) {
          this.loadError.set(true)
        }
      },
    })
  }

  protected retryLoad(): void {
    this.resetForNewUser()
    this.reload()
  }

  openRoleEditor(entry: UserPermissionEntry): void {
    this.editingPermission.set(entry)
  }

  closeRoleEditor(): void {
    this.editingPermission.set(null)
  }

  changeRole(role: Role): void {
    const entry = this.editingPermission()
    if (!entry || this.savingRole()) {
      return
    }
    this.savingRole.set(true)
    const userId = this.userId
    this.permissionsService.grant(entry.repository_id, userId, role).subscribe({
      next: () => {
        this.toastService.success(t('users.detail.updated'))
        if (userId !== this.userId) {
          return
        }
        this.savingRole.set(false)
        this.editingPermission.set(null)
        this.reload()
      },
      error: () => {
        if (userId === this.userId) {
          this.savingRole.set(false)
        }
        this.toastService.error(t('users.detail.errors.updateFailed'))
      },
    })
  }

  revokeFromEditor(): void {
    const entry = this.editingPermission()
    if (!entry || this.savingRole()) {
      return
    }
    this.savingRole.set(true)
    const userId = this.userId
    this.permissionsService.revoke(entry.repository_id, userId).subscribe({
      next: () => {
        this.toastService.success(t('users.detail.revoked'))
        if (userId !== this.userId) {
          return
        }
        this.savingRole.set(false)
        this.editingPermission.set(null)
        this.reload()
      },
      error: () => {
        if (userId === this.userId) {
          this.savingRole.set(false)
        }
        this.toastService.error(t('users.detail.errors.revokeFailed'))
      },
    })
  }

  /** From a row's menu: asks first, then revokes as the editor would. */
  /** The old password stops working at once; the link goes by mail, or comes back when the mail cannot go out. */
  async resetPassword(): Promise<void> {
    const user = this.user()
    if (!user || this.resettingPassword()) {
      return
    }
    const confirmed = await this.confirmService.ask({
      heading: t('users.passwordReset.heading'),
      message: t('users.passwordReset.message', { username: displayName(user) }),
      confirmLabel: t('users.passwordReset.confirm'),
      danger: true,
    })
    if (!confirmed) {
      return
    }
    this.resettingPassword.set(true)
    this.usersService.resetPassword(user.id).subscribe({
      next: (mail) => {
        this.resettingPassword.set(false)
        if (mail.email_sent) {
          this.mailFailure.set(null)
          this.toastService.success(t('users.passwordReset.sent', { username: displayName(user) }))
          return
        }
        this.mailFailure.set({ mail, kind: 'password-reset' })
        afterNextRender(() => this.mailFailedAlert()?.focus(), { injector: this.injector })
      },
      error: (error: unknown) => {
        this.resettingPassword.set(false)
        this.toastService.error(resetPasswordFailure(error))
      },
    })
  }

  /** For someone who lost every factor: signed out everywhere, they set one up again at their next sign-in. */
  async resetMfa(): Promise<void> {
    const user = this.user()
    if (!user || this.resettingMfa()) {
      return
    }
    const confirmed = await this.confirmService.ask({
      heading: t('users.mfaReset.heading'),
      message: resetMfaMessage(displayName(user), false),
      confirmLabel: t('users.mfaReset.confirm'),
      danger: true,
    })
    if (!confirmed) {
      return
    }
    this.resettingMfa.set(true)
    this.usersService.resetMfa(user.id).subscribe({
      next: () => {
        this.resettingMfa.set(false)
        this.toastService.success(t('users.mfaReset.done'))
        this.reload()
      },
      error: (error: unknown) => {
        this.resettingMfa.set(false)
        this.toastService.error(resetMfaFailure(error))
      },
    })
  }

  async revokePermission(entry: UserPermissionEntry): Promise<void> {
    const confirmed = await this.confirmService.ask({
      heading: t('users.detail.revokeHeading'),
      message: t('users.detail.revokeMessage', {
        username: this.username(),
        repository: entry.repository_name,
      }),
      confirmLabel: t('users.detail.revokeAction'),
      danger: true,
    })
    if (!confirmed) {
      return
    }
    this.editingPermission.set(entry)
    this.revokeFromEditor()
  }

  grantPermission(): void {
    const repositoryIds = this.grantRepositoryIds()
    if (repositoryIds.length === 0 || this.granting()) {
      return
    }
    const role = this.grantRole()
    const userId = this.userId
    this.granting.set(true)
    forkJoin(repositoryIds.map((id) => this.permissionsService.grant(id, userId, role))).subscribe({
      next: () => {
        this.toastService.success(t('users.detail.granted'))
        if (userId === this.userId) {
          this.granting.set(false)
          this.grantRepositoryIds.set([])
          this.reload()
        }
      },
      error: () => {
        // forkJoin reports only the first failure, but earlier grants may have landed.
        this.toastService.error(t('users.detail.errors.grantFailed'))
        if (userId === this.userId) {
          this.granting.set(false)
          this.reload()
        }
      },
    })
  }

  /** Granting is immediate; removing asks first, since it takes the administration away. */
  async setSuperAdmin(): Promise<void> {
    const user = this.user()
    if (!user || this.settingSuperAdmin()) {
      return
    }
    const next = !user.is_super_admin
    if (!next) {
      const confirmed = await this.confirmService.ask({
        heading: t('users.detail.demoteHeading'),
        message: t('users.list.demoteMessage', { username: displayName(user) }),
        confirmLabel: t('users.detail.demoteAction'),
        danger: true,
      })
      if (!confirmed) {
        return
      }
    }
    this.settingSuperAdmin.set(true)
    this.usersService.setSuperAdmin(user.id, next).subscribe({
      next: () => {
        this.settingSuperAdmin.set(false)
        this.reload()
        this.toastService.success(
          next
            ? t('users.detail.promoted', { username: displayName(user) })
            : t('users.detail.demoted', { username: displayName(user) }),
        )
      },
      error: (err: HttpErrorResponse) => {
        this.settingSuperAdmin.set(false)
        this.toastService.error(
          err.status === 409
            ? t('users.detail.errors.lastSuperAdmin')
            : t('users.detail.errors.superAdminStatus'),
        )
      },
    })
  }

  resendInvitation(): void {
    const user = this.user()
    if (!user || this.resendingInvitation()) {
      return
    }
    this.resendingInvitation.set(true)
    this.usersService.resendInvitation(user.id).subscribe({
      next: (mail) => {
        this.resendingInvitation.set(false)
        this.reload()
        if (mail.email_sent) {
          this.mailFailure.set(null)
          this.toastService.success(
            t('users.list.resent', { email: user.email ?? displayName(user) }),
          )
          return
        }
        // The old link no longer works: the new one is the only way in.
        this.mailFailure.set({ mail, kind: 'invitation' })
        afterNextRender(() => this.mailFailedAlert()?.focus(), { injector: this.injector })
      },
      error: () => {
        this.resendingInvitation.set(false)
        this.toastService.error(t('users.detail.errors.resendFailed'))
      },
    })
  }

  async deleteUser(): Promise<void> {
    const user = this.user()
    if (!user) {
      return
    }
    const confirmed = await this.confirmService.ask({
      heading: t('users.detail.delete'),
      message: t('users.detail.deleteConfirm', { username: displayName(user) }),
      confirmLabel: t('users.detail.delete'),
      danger: true,
      typeToConfirm: displayName(user),
    })
    if (!confirmed) {
      return
    }
    this.usersService.delete(user.id).subscribe({
      next: () => {
        this.router.navigate(['/users'])
        this.toastService.success(t('users.detail.deleted', { username: displayName(user) }))
      },
      error: (err: unknown) =>
        this.toastService.error(
          err instanceof HttpErrorResponse && err.status === 409
            ? t('users.detail.errors.deleteLastSuperAdmin')
            : t('users.detail.errors.deleteFailed'),
        ),
    })
  }
}
