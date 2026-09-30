import { t } from '../../shared/i18n/translator'
import { TranslocoPipe } from '@jsverse/transloco'
import { ChangeDetectionStrategy, Component, computed, effect, inject, signal } from '@angular/core'
import { toSignal } from '@angular/core/rxjs-interop'
import { ActivatedRoute, Router } from '@angular/router'
import { HttpErrorResponse } from '@angular/common/http'
import { FormsModule } from '@angular/forms'
import { catchError, forkJoin, map, of } from 'rxjs'
import { Button, Card, Select, Table, TableColumn, Tooltip } from '@masmarino/gabarit'
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
import { UserSummary } from '../domain/user.entity'
import { formatSelectedCount } from '../../shared/format'
import { MeService } from '../../shell/application/me.service'
import { ToastService } from '../../shared/toast.service'
import { ConfirmService } from '../../shared/confirm.service'

@Component({
  selector: 'app-user-detail',
  standalone: true,
  imports: [TranslocoPipe, Table, Button, Select, PermissionRoleEditor, FormsModule, Card, Tooltip],
  templateUrl: './user-detail.html',
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
  readonly selectedCountLabel = formatSelectedCount
  readonly username = computed(() => this.user()?.username ?? '')

  // Super-admin status is not an organization-scoped right.
  readonly isSuperAdminViewer = computed(() => this.me.isSuperAdmin())
  // Hidden for a super-admin target: the backend 403s an org admin there.
  readonly canManageTarget = computed(
    () => this.isSuperAdminViewer() || !this.user()?.is_super_admin,
  )

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
  readonly permissions = signal<UserPermissionEntry[]>([])
  readonly editingPermission = signal<UserPermissionEntry | null>(null)
  readonly settingSuperAdmin = signal(false)
  readonly resendingInvitation = signal(false)

  readonly repositories = signal<RepositorySummary[]>([])
  readonly repositoryOptions = computed(() =>
    this.repositories().map((repo) => ({ value: repo.id, label: repo.name })),
  )
  readonly grantRepositoryIds = signal<string[]>([])
  readonly grantRole = signal<Role>('read')
  readonly roleOptions = ROLE_OPTIONS

  readonly permissionColumns: TableColumn<UserPermissionEntry>[] = [
    { key: 'repository_name', label: t('users.detail.columns.repository') },
    { key: 'format', label: t('common.format') },
    { key: 'role', label: t('users.detail.role') },
  ]
  readonly permissionRowId = (p: UserPermissionEntry): string => p.repository_id

  // Reused across navigations: drop the previous data.
  private resetForNewUser(): void {
    this.user.set(null)
    this.permissions.set([])
    this.editingPermission.set(null)
    this.grantRepositoryIds.set([])
    this.savingRole.set(false)
    this.settingSuperAdmin.set(false)
    this.resendingInvitation.set(false)
  }

  private reload(): void {
    const requestedId = this.userId
    this.loadError.set(false)
    this.usersService
      .get(requestedId)
      .pipe(
        catchError(() => {
          if (requestedId === this.userId) {
            this.loadError.set(true)
          }
          return of(null)
        }),
      )
      .subscribe((user) => {
        if (requestedId === this.userId) {
          this.user.set(user)
        }
      })
    this.permissionsService.listForUser(requestedId).subscribe({
      next: (permissions) => {
        if (requestedId === this.userId) {
          this.permissions.set(permissions)
        }
      },
      error: () => {
        if (requestedId === this.userId) {
          this.loadError.set(true)
        }
      },
    })
  }

  openRoleEditor(entry: UserPermissionEntry): void {
    this.editingPermission.set(entry)
  }

  closeRoleEditor(): void {
    this.editingPermission.set(null)
  }

  readonly savingRole = signal(false)

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

  grantPermission(): void {
    const repositoryIds = this.grantRepositoryIds()
    if (repositoryIds.length === 0) {
      return
    }
    const role = this.grantRole()
    const userId = this.userId
    forkJoin(repositoryIds.map((id) => this.permissionsService.grant(id, userId, role))).subscribe({
      next: () => {
        this.toastService.success(t('users.detail.granted'))
        if (userId === this.userId) {
          this.grantRepositoryIds.set([])
          this.reload()
        }
      },
      error: () => {
        // forkJoin reports only the first failure, but earlier grants may have landed.
        this.toastService.error(t('users.detail.errors.grantFailed'))
        if (userId === this.userId) {
          this.reload()
        }
      },
    })
  }

  async setSuperAdmin(): Promise<void> {
    const user = this.user()
    if (!user || this.settingSuperAdmin()) {
      return
    }
    const next = !user.is_super_admin
    const message = next
      ? t('users.detail.promoteConfirm', { username: user.username })
      : t('users.detail.demoteConfirm', { username: user.username })
    const confirmed = await this.confirmService.ask({
      heading: next ? t('users.detail.promoteHeading') : t('users.detail.demoteHeading'),
      message,
      confirmLabel: next ? t('users.detail.promoteAction') : t('users.detail.demoteAction'),
      danger: !next,
    })
    if (!confirmed) {
      return
    }
    this.settingSuperAdmin.set(true)
    this.usersService.setSuperAdmin(user.id, next).subscribe({
      next: () => {
        this.settingSuperAdmin.set(false)
        this.reload()
        this.toastService.success(
          next
            ? t('users.detail.promoted', { username: user.username })
            : t('users.detail.demoted', { username: user.username }),
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
    if (!user) {
      return
    }
    this.resendingInvitation.set(true)
    this.usersService.resendInvitation(user.id).subscribe({
      next: () => {
        this.resendingInvitation.set(false)
        this.toastService.success(t('users.detail.resent'))
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
      message: t('users.detail.deleteConfirm', { username: user.username }),
      confirmLabel: t('common.delete'),
      danger: true,
      typeToConfirm: user.username,
    })
    if (!confirmed) {
      return
    }
    this.usersService.delete(user.id).subscribe({
      next: () => {
        this.router.navigate(['/users'])
        this.toastService.success(t('users.detail.deleted', { username: user.username }))
      },
      error: () => this.toastService.error(t('users.detail.errors.deleteFailed')),
    })
  }
}
