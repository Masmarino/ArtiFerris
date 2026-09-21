import { ChangeDetectionStrategy, Component, computed, effect, inject, signal } from '@angular/core'
import { toSignal } from '@angular/core/rxjs-interop'
import { ActivatedRoute, Router } from '@angular/router'
import { FormsModule } from '@angular/forms'
import { catchError, map, of } from 'rxjs'
import {
  Autocomplete,
  Button,
  Card, Divider,
  GbtInput,
  Select,
  Tab,
  Table,
  TableColumn,
  Tabs,
} from '@masmarino/gabarit'
import { RepositoriesService } from '../application/repositories.service'
import { RepositorySummary } from '../domain/repository.entity'
import { PermissionsService } from '../application/permissions.service'
import { PermissionEntry, ROLE_OPTIONS, Role, UserLookup } from '../domain/permission.entity'
import { UsageInstructions } from '../usage-instructions/usage-instructions'
import { PermissionRoleEditor } from '../permission-role-editor/permission-role-editor'
import { PackageTree } from '../package-tree/package-tree'
import { PageTitleService } from '../../shell/page-title.service'
import { FormatBytesPipe } from '../../shared/format-bytes.pipe'
import { formatResultsAnnouncement } from '../../shared/format'
import { ToastService } from '../../shared/toast.service'

const BYTES_PER_MB = 1024 * 1024

@Component({
  selector: 'app-repository-detail',
  standalone: true,
  imports: [
    Table,
    Button,
    GbtInput,
    Autocomplete,
    Select,
    Tab,
    Tabs,
    FormsModule,
    UsageInstructions,
    PermissionRoleEditor,
    PackageTree,
    Card,
    FormatBytesPipe,
    Divider,
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
  private readonly pageTitle = inject(PageTitleService)

  // Reactive, not route.snapshot — Angular reuses this component across :id navigations.
  private readonly routeId = toSignal(
    this.route.paramMap.pipe(map((params) => params.get('id')!)),
    {
      requireSync: true,
    },
  )

  readonly repository = signal<RepositorySummary | null>(null)
  readonly loadError = signal(false)
  readonly repositoryName = computed(() => this.repository()?.name ?? '')
  // Admin-only actions are hidden, not just left to fail with a 403.
  readonly isAdmin = computed(() => this.repository()?.my_role === 'admin')
  readonly groupMemberRows = computed(
    () => this.repository()?.group_members.map((id) => ({ id })) ?? [],
  )

  constructor() {
    effect(() => this.pageTitle.title.set(this.repositoryName()))
    effect(() => this.reload(this.routeId()))
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

  // Lets the group-members table show names instead of raw ids.
  readonly repositoryNamesById = signal<Map<string, string>>(new Map())

  readonly memberColumns: TableColumn<{ id: string }>[] = [
    {
      key: 'id',
      label: 'Dépôt membre',
      format: (row) => this.repositoryNamesById().get(row.id) ?? row.id,
    },
  ]
  readonly memberRowId = (row: { id: string }): string => row.id

  readonly permissionColumns: TableColumn<PermissionEntry>[] = [
    { key: 'username', label: 'Utilisateur' },
    { key: 'role', label: 'Rôle' },
  ]
  readonly permissionRowId = (p: PermissionEntry): string => p.user_id

  private reload(id: string): void {
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
      })
    this.permissionsService.list(id).subscribe((permissions) => {
      if (id !== this.routeId()) {
        return
      }
      this.permissions.set(permissions)
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
          this.quotaSaved.set(true)
          this.reload(repository.id)
        },
        error: () => {
          this.savingQuota.set(false)
          this.quotaError.set("Échec de l'enregistrement du quota.")
        },
      })
      return
    }
    const mb = Number(raw)
    if (!Number.isFinite(mb) || mb < 0) {
      this.quotaError.set('Doit être un nombre positif (ou vide pour illimité).')
      return
    }
    this.savingQuota.set(true)
    this.repositoriesService.setQuota(repository.id, Math.round(mb * BYTES_PER_MB)).subscribe({
      next: () => {
        this.savingQuota.set(false)
        this.quotaSaved.set(true)
        this.reload(repository.id)
      },
      error: () => {
        this.savingQuota.set(false)
        this.quotaError.set("Échec de l'enregistrement du quota.")
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
      this.retentionError.set('Doit être un entier positif (ou vide pour désactiver).')
      return
    }
    this.savingRetention.set(true)
    this.repositoriesService.setRetentionPolicy(repository.id, keepLastN).subscribe({
      next: () => {
        this.savingRetention.set(false)
        this.retentionSaved.set(true)
        this.reload(repository.id)
      },
      error: () => {
        this.savingRetention.set(false)
        this.retentionError.set("Échec de l'enregistrement de la politique de rétention.")
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
        this.toastService.success("Droit d'accès accordé.")
      },
      error: () => {
        this.grantingPermission.set(false)
        this.toastService.error("Échec de l'attribution du droit d'accès.")
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
        this.toastService.success("Droit d'accès mis à jour.")
      },
      error: () => {
        this.savingRole.set(false)
        this.toastService.error("Échec de la mise à jour du droit d'accès.")
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
        this.toastService.success("Droit d'accès révoqué.")
      },
      error: () => {
        this.savingRole.set(false)
        this.toastService.error("Échec de la révocation du droit d'accès.")
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
        this.toastService.success('Dépôt renommé.')
      },
      error: () => {
        this.renaming.set(false)
        this.toastService.error('Échec du renommage du dépôt.')
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
          this.toastService.success('Dépôt membre ajouté.')
        },
        error: () => {
          this.addingMember.set(false)
          this.toastService.error("Échec de l'ajout du dépôt membre.")
        },
      })
  }

  removeMember(memberId: string): void {
    const repository = this.repository()
    if (!repository) {
      return
    }
    const memberName = this.repositoryNamesById().get(memberId) ?? memberId
    if (!confirm(`Retirer "${memberName}" du groupe ?`)) {
      return
    }
    this.repositoriesService.removeGroupMember(repository.id, memberId).subscribe({
      next: () => {
        this.reload(repository.id)
        this.toastService.success(`« ${memberName} » retiré du groupe.`)
      },
      error: () => this.toastService.error('Échec du retrait du dépôt membre.'),
    })
  }

  readonly deletingRepository = signal(false)

  deleteRepository(): void {
    const repository = this.repository()
    if (
      !repository ||
      this.deletingRepository() ||
      !confirm(`Supprimer le dépôt "${repository.name}" ?`)
    ) {
      return
    }
    this.deletingRepository.set(true)
    this.repositoriesService.delete(repository.id).subscribe({
      next: () => {
        this.router.navigate(['/repositories'])
        this.toastService.success(`Dépôt « ${repository.name} » supprimé.`)
      },
      error: () => {
        this.deletingRepository.set(false)
        this.toastService.error('Échec de la suppression du dépôt.')
      },
    })
  }
}
