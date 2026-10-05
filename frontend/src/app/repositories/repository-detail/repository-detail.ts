import { t } from '../../shared/i18n/translator'
import { TranslocoPipe } from '@jsverse/transloco'
import { HttpErrorResponse } from '@angular/common/http'
import { ChangeDetectionStrategy, Component, computed, effect, inject, signal } from '@angular/core'
import { toSignal } from '@angular/core/rxjs-interop'
import { ActivatedRoute, Router } from '@angular/router'
import { FormsModule } from '@angular/forms'
import { catchError, map, of } from 'rxjs'
import { Autocomplete } from '@masmarino/gabarit/autocomplete'
import { Button } from '@masmarino/gabarit/button'
import { Card } from '@masmarino/gabarit/card'
import { Divider } from '@masmarino/gabarit/divider'
import { EmptyState } from '@masmarino/gabarit/empty-state'
import { GbtInput } from '@masmarino/gabarit/input'
import { Select } from '@masmarino/gabarit/select'
import { Table, TableColumn } from '@masmarino/gabarit/table'
import { Tab, Tabs } from '@masmarino/gabarit/tabs'
import { RepositoriesService } from '../application/repositories.service'
import { RepositorySummary } from '../domain/repository.entity'
import { PermissionsService } from '../application/permissions.service'
import { PermissionEntry, ROLE_OPTIONS, Role, UserLookup } from '../domain/permission.entity'
import { UsageInstructions } from '../usage-instructions/usage-instructions'
import { ShareRepositoryLink } from '../share-repository-link/share-repository-link'
import { PermissionRoleEditor } from '../permission-role-editor/permission-role-editor'
import { PackageTree } from '../package-tree/package-tree'
import { PageTitleService } from '../../shell/page-title.service'
import { FormatBytesPipe } from '../../shared/format-bytes.pipe'
import { formatResultsAnnouncement } from '../../shared/format'
import { ConfirmService } from '../../shared/confirm.service'
import { MeService } from '../../shell/application/me.service'
import { ToastService } from '../../shared/toast.service'
import { PageHeading } from '../../shared/page-heading/page-heading'

const BYTES_PER_MB = 1024 * 1024

@Component({
  selector: 'app-repository-detail',
  standalone: true,
  imports: [
    PageHeading,
    TranslocoPipe,
    Table,
    Button,
    GbtInput,
    Autocomplete,
    Select,
    Tab,
    Tabs,
    FormsModule,
    UsageInstructions,
    ShareRepositoryLink,
    PermissionRoleEditor,
    PackageTree,
    Card,
    FormatBytesPipe,
    Divider,
    EmptyState,
  ],
  templateUrl: './repository-detail.html',
  styleUrl: './repository-detail.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class RepositoryDetail {
  private readonly route = inject(ActivatedRoute)
  private readonly router = inject(Router)
  private readonly repositoriesService = inject(RepositoriesService)
  private readonly permissionsService = inject(PermissionsService)
  private readonly toastService = inject(ToastService)
  private readonly confirmService = inject(ConfirmService)
  private readonly me = inject(MeService)
  private readonly pageTitle = inject(PageTitleService)

  // Reactive, not route.snapshot: Angular reuses this component across navigations.
  private readonly routeId = toSignal(
    this.route.paramMap.pipe(map((params) => params.get('id')!)),
    {
      requireSync: true,
    },
  )

  readonly repository = signal<RepositorySummary | null>(null)
  readonly loadError = signal(false)
  readonly repositoryName = computed(() => this.repository()?.name ?? '')
  // Admin-only actions are hidden, not left to fail with a 403.
  readonly isAdmin = computed(() => this.repository()?.my_role === 'admin')
  readonly groupMemberRows = computed(
    () => this.repository()?.group_members.map((id) => ({ id })) ?? [],
  )

  constructor() {
    effect(() => this.pageTitle.title.set(this.repositoryName()))
    effect(() => {
      const id = this.routeId()
      this.resetForNewRepository()
      this.reload(id)
    })
  }
  readonly newMemberId = signal('')
  readonly newName = signal('')

  readonly quotaMb = signal('')
  readonly quotaError = signal<string | null>(null)
  readonly quotaSaved = signal(false)

  readonly retentionKeepLastN = signal('')
  readonly retentionError = signal<string | null>(null)
  readonly retentionSaved = signal(false)

  readonly permissions = signal<PermissionEntry[]>([])
  readonly grantUser = signal<UserLookup | null>(null)
  readonly grantRole = signal<Role>('read')
  readonly roleOptions = ROLE_OPTIONS
  readonly editingPermission = signal<PermissionEntry | null>(null)

  readonly repositoryNamesById = signal<Map<string, string>>(new Map())

  readonly memberColumns: TableColumn<{ id: string }>[] = [
    {
      key: 'id',
      label: t('repositories.detail.columns.member'),
      format: (row) => this.repositoryNamesById().get(row.id) ?? row.id,
    },
  ]
  readonly memberRowId = (row: { id: string }): string => row.id

  readonly permissionColumns: TableColumn<PermissionEntry>[] = [
    { key: 'username', label: t('repositories.detail.columns.user') },
    { key: 'role', label: t('repositories.permissions.role') },
  ]
  readonly permissionRowId = (p: PermissionEntry): string => p.user_id

  // Reused across navigations: drop the previous data.
  private resetForNewRepository(): void {
    this.repository.set(null)
    this.permissions.set([])
    this.repositoryNamesById.set(new Map())
    this.editingPermission.set(null)
    this.grantUser.set(null)
    this.grantRole.set('read')
    this.newMemberId.set('')
    this.newName.set('')
    this.quotaMb.set('')
    this.quotaError.set(null)
    this.quotaSaved.set(false)
    this.retentionKeepLastN.set('')
    this.retentionError.set(null)
    this.retentionSaved.set(false)
    this.savingQuota.set(false)
    this.savingRetention.set(false)
    this.grantingPermission.set(false)
    this.savingRole.set(false)
    this.renaming.set(false)
    this.addingMember.set(false)
    this.changingVisibility.set(false)
    this.deletingRepository.set(false)
  }

  private reload(id: string): void {
    if (id !== this.routeId()) {
      return
    }
    this.loadError.set(false)
    this.repositoriesService
      .get(id)
      .pipe(
        catchError(() => {
          if (id === this.routeId()) {
            this.loadError.set(true)
          }
          return of(null)
        }),
      )
      .subscribe((repository) => {
        if (id !== this.routeId() || !repository) {
          return
        }
        this.repository.set(repository)
        this.newName.set(repository.name)
        this.quotaMb.set(
          repository.quota_bytes == null ? '' : String(repository.quota_bytes / BYTES_PER_MB),
        )
        this.retentionKeepLastN.set(
          repository.retention_keep_last_n == null ? '' : String(repository.retention_keep_last_n),
        )
        if (repository.repo_type === 'group' && repository.group_members.length > 0) {
          this.repositoriesService.list().subscribe((repositories) => {
            if (id !== this.routeId()) {
              return
            }
            this.repositoryNamesById.set(new Map(repositories.map((r) => [r.id, r.name])))
          })
        }
        // A viewer with only implicit read on a public repository is refused this list (403/404):
        // expected, it must not block the page.
        // A real server error still does.
        this.permissionsService.list(id).subscribe({
          next: (permissions) => {
            if (id === this.routeId()) {
              this.permissions.set(permissions)
            }
          },
          error: (error: HttpErrorResponse) => {
            if (id === this.routeId() && error.status !== 403 && error.status !== 404) {
              this.loadError.set(true)
            }
          },
        })
      })
  }

  setQuotaMb(value: string): void {
    this.quotaMb.set(value)
    this.quotaSaved.set(false)
  }

  readonly savingQuota = signal(false)

  saveQuota(): void {
    const repository = this.repository()
    if (!repository || this.savingQuota()) {
      return
    }
    this.quotaError.set(null)
    this.quotaSaved.set(false)
    const raw = this.quotaMb().trim()
    if (raw === '') {
      this.savingQuota.set(true)
      this.repositoriesService.setQuota(repository.id, null).subscribe({
        next: () => {
          this.savingQuota.set(false)
          if (repository.id === this.routeId()) {
            this.quotaSaved.set(true)
          }
          this.reload(repository.id)
        },
        error: () => {
          this.savingQuota.set(false)
          if (repository.id === this.routeId()) {
            this.quotaError.set(t('repositories.detail.errors.quotaSave'))
          }
        },
      })
      return
    }
    const mb = Number(raw)
    if (!Number.isFinite(mb) || mb < 0) {
      this.quotaError.set(t('repositories.create.errors.quota'))
      return
    }
    this.savingQuota.set(true)
    this.repositoriesService.setQuota(repository.id, Math.round(mb * BYTES_PER_MB)).subscribe({
      next: () => {
        this.savingQuota.set(false)
        if (repository.id === this.routeId()) {
          this.quotaSaved.set(true)
        }
        this.reload(repository.id)
      },
      error: () => {
        this.savingQuota.set(false)
        if (repository.id === this.routeId()) {
          this.quotaError.set(t('repositories.detail.errors.quotaSave'))
        }
      },
    })
  }

  setRetentionKeepLastN(value: string): void {
    this.retentionKeepLastN.set(value)
    this.retentionSaved.set(false)
  }

  readonly savingRetention = signal(false)

  saveRetentionPolicy(): void {
    const repository = this.repository()
    if (!repository || this.savingRetention()) {
      return
    }
    this.retentionError.set(null)
    this.retentionSaved.set(false)
    const raw = this.retentionKeepLastN().trim()
    const keepLastN = raw === '' ? null : Number(raw)
    if (keepLastN !== null && (!Number.isInteger(keepLastN) || keepLastN < 1)) {
      this.retentionError.set(t('repositories.create.errors.retention'))
      return
    }
    this.savingRetention.set(true)
    this.repositoriesService.setRetentionPolicy(repository.id, keepLastN).subscribe({
      next: () => {
        this.savingRetention.set(false)
        if (repository.id === this.routeId()) {
          this.retentionSaved.set(true)
        }
        this.reload(repository.id)
      },
      error: () => {
        this.savingRetention.set(false)
        if (repository.id === this.routeId()) {
          this.retentionError.set(t('repositories.detail.errors.retentionSave'))
        }
      },
    })
  }

  readonly userDisplayFn = (candidate: UserLookup): string => candidate.username
  readonly resultsAnnouncement = formatResultsAnnouncement
  readonly searchUsers = (query: string) => this.permissionsService.searchUsers(query)

  readonly grantingPermission = signal(false)

  grantPermission(): void {
    const repository = this.repository()
    const user = this.grantUser()
    if (!repository || !user || this.grantingPermission()) {
      return
    }
    this.grantingPermission.set(true)
    this.permissionsService.grant(repository.id, user.id, this.grantRole()).subscribe({
      next: () => {
        this.grantingPermission.set(false)
        this.grantUser.set(null)
        this.reload(repository.id)
        this.toastService.success(t('users.detail.granted'))
      },
      error: () => {
        this.grantingPermission.set(false)
        this.toastService.error(t('repositories.detail.errors.grantFailed'))
      },
    })
  }

  openRoleEditor(entry: PermissionEntry): void {
    this.editingPermission.set(entry)
  }

  closeRoleEditor(): void {
    this.editingPermission.set(null)
  }

  readonly savingRole = signal(false)

  changeRole(role: Role): void {
    const repository = this.repository()
    const entry = this.editingPermission()
    if (!repository || !entry || this.savingRole()) {
      return
    }
    this.savingRole.set(true)
    this.permissionsService.grant(repository.id, entry.user_id, role).subscribe({
      next: () => {
        this.savingRole.set(false)
        this.editingPermission.set(null)
        this.reload(repository.id)
        this.toastService.success(t('users.detail.updated'))
      },
      error: () => {
        this.savingRole.set(false)
        this.toastService.error(t('users.detail.errors.updateFailed'))
      },
    })
  }

  revokeFromEditor(): void {
    const repository = this.repository()
    const entry = this.editingPermission()
    if (!repository || !entry || this.savingRole()) {
      return
    }
    this.savingRole.set(true)
    this.permissionsService.revoke(repository.id, entry.user_id).subscribe({
      next: () => {
        this.savingRole.set(false)
        this.editingPermission.set(null)
        this.reload(repository.id)
        this.toastService.success(t('users.detail.revoked'))
      },
      error: () => {
        this.savingRole.set(false)
        this.toastService.error(t('users.detail.errors.revokeFailed'))
      },
    })
  }

  readonly renaming = signal(false)

  rename(): void {
    const repository = this.repository()
    if (!repository || !this.newName() || this.newName() === repository.name || this.renaming()) {
      return
    }
    this.renaming.set(true)
    this.repositoriesService.rename(repository.id, this.newName()).subscribe({
      next: () => {
        this.renaming.set(false)
        this.reload(repository.id)
        this.toastService.success(t('repositories.detail.renamed'))
      },
      error: () => {
        this.renaming.set(false)
        this.toastService.error(t('repositories.detail.errors.renameFailed'))
      },
    })
  }

  readonly addingMember = signal(false)

  addMember(): void {
    const repository = this.repository()
    if (!repository || !this.newMemberId() || this.addingMember()) {
      return
    }
    this.addingMember.set(true)
    this.repositoriesService
      .addGroupMember(repository.id, this.newMemberId(), repository.group_members.length)
      .subscribe({
        next: () => {
          this.addingMember.set(false)
          this.newMemberId.set('')
          this.reload(repository.id)
          this.toastService.success(t('repositories.detail.memberAdded'))
        },
        error: () => {
          this.addingMember.set(false)
          this.toastService.error(t('repositories.detail.errors.memberAddFailed'))
        },
      })
  }

  async removeMember(memberId: string): Promise<void> {
    const repository = this.repository()
    if (!repository) {
      return
    }
    const memberName = this.repositoryNamesById().get(memberId) ?? memberId
    const confirmed = await this.confirmService.ask({
      heading: t('repositories.detail.removeFromGroup'),
      message: t('repositories.detail.removeFromGroupMessage', { name: memberName }),
      confirmLabel: t('repositories.create.remove'),
      danger: true,
    })
    if (!confirmed) {
      return
    }
    this.repositoriesService.removeGroupMember(repository.id, memberId).subscribe({
      next: () => {
        this.reload(repository.id)
        this.toastService.success(t('repositories.detail.memberRemoved', { name: memberName }))
      },
      error: () => this.toastService.error(t('repositories.detail.errors.memberRemoveFailed')),
    })
  }

  readonly canChangeVisibility = computed(() => {
    const repository = this.repository()
    return repository?.repo_type === 'hosted' && (this.isAdmin() || this.me.isSuperAdmin())
  })

  readonly changingVisibility = signal(false)

  async changeVisibility(): Promise<void> {
    const repository = this.repository()
    if (!repository || this.changingVisibility()) {
      return
    }
    const makePublic = !repository.is_public
    const confirmed = await this.confirmService.ask(
      makePublic
        ? {
            heading: t('repositories.detail.makePublicHeading'),
            message: t('repositories.detail.makePublicMessage', { name: repository.name }),
            confirmLabel: t('repositories.detail.makePublic'),
            danger: true,
          }
        : {
            heading: t('repositories.detail.makePrivateHeading'),
            message: t('repositories.detail.makePrivateMessage', { name: repository.name }),
            confirmLabel: t('repositories.detail.makePrivate'),
          },
    )
    if (!confirmed) {
      return
    }
    this.changingVisibility.set(true)
    this.repositoriesService.setVisibility(repository.id, makePublic).subscribe({
      next: () => {
        this.changingVisibility.set(false)
        this.toastService.success(
          t(makePublic ? 'repositories.detail.nowPublic' : 'repositories.detail.nowPrivate', {
            name: repository.name,
          }),
        )
        this.reload(repository.id)
      },
      error: () => {
        this.changingVisibility.set(false)
        this.toastService.error(t('repositories.detail.errors.visibilityFailed'))
      },
    })
  }

  readonly deletingRepository = signal(false)

  async deleteRepository(): Promise<void> {
    const repository = this.repository()
    if (!repository || this.deletingRepository()) {
      return
    }
    const confirmed = await this.confirmService.ask({
      heading: t('repositories.detail.delete'),
      message: t('repositories.detail.deleteMessage', { name: repository.name }),
      confirmLabel: t('common.delete'),
      danger: true,
      typeToConfirm: repository.name,
    })
    if (!confirmed) {
      return
    }
    this.deletingRepository.set(true)
    this.repositoriesService.delete(repository.id).subscribe({
      next: () => {
        this.router.navigate(['/repositories'])
        this.toastService.success(t('repositories.detail.deleted', { name: repository.name }))
      },
      error: () => {
        this.deletingRepository.set(false)
        this.toastService.error(t('repositories.detail.errors.deleteFailed'))
      },
    })
  }
}
