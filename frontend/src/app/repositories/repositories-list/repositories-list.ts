import {
  ChangeDetectionStrategy,
  Component,
  OnInit,
  computed,
  inject,
  input,
  signal,
} from '@angular/core'
import { Router } from '@angular/router'
import { FormsModule } from '@angular/forms'
import { forkJoin } from 'rxjs'
import {
  Button,
  EmptyState,
  Select,
  SelectOption,
  Spinner,
  Table,
  TableColumn,
} from '@masmarino/gabarit'
import { CreateRepositoryModal } from '../create-repository-modal/create-repository-modal'
import { CreateUserProjectModal } from '../create-user-project-modal/create-user-project-modal'
import { RepositoriesService } from '../application/repositories.service'
import { PersonalRepositoryService } from '../application/personal-repository.service'
import { RepositorySummary } from '../domain/repository.entity'
import { OrganizationsService } from '../../admin/application/organizations.service'
import { OrganizationSummary } from '../../admin/domain/organization.entity'
import { MeService } from '../../shell/application/me.service'

/** Not a real organization id — selects the unfiltered view across every organization. */
const ALL_ORGANIZATIONS = 'ALL'

@Component({
  selector: 'app-repositories-list',
  standalone: true,
  imports: [
    Table,
    Button,
    EmptyState,
    Select,
    Spinner,
    FormsModule,
    CreateRepositoryModal,
    CreateUserProjectModal,
  ],
  templateUrl: './repositories-list.html',
  styleUrl: './repositories-list.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class RepositoriesList implements OnInit {
  private readonly repositoriesService = inject(RepositoriesService)
  private readonly personalRepositoryService = inject(PersonalRepositoryService)
  private readonly organizationsService = inject(OrganizationsService)
  private readonly me = inject(MeService)
  private readonly router = inject(Router)

  // 'personal' is the caller's own personal-namespace view (see PersonalRepositoryService) —
  // there's only ever one organization in it, so every organization-scoped control below is
  // meaningless there regardless of the viewer's own role.
  readonly mode = input<'organization' | 'personal'>('organization')

  // Gates the organization filter/column — a super-admin is the only one who ever sees
  // repositories across more than their own organization (see list_repositories's backend
  // comment), so those controls would be meaningless for anyone else.
  readonly isSuperAdmin = computed(() => this.me.isSuperAdmin())

  // Also requires 'organization' mode — see the comment on `mode` above.
  readonly showOrganizationControls = computed(
    () => this.mode() === 'organization' && this.isSuperAdmin(),
  )

  readonly repositories = signal<RepositorySummary[]>([])
  readonly organizations = signal<OrganizationSummary[]>([])
  readonly selectedOrganizationId = signal<string>(ALL_ORGANIZATIONS)
  readonly showCreateModal = signal(false)
  readonly loading = signal(true)
  readonly error = signal<string | null>(null)

  private readonly organizationNamesById = computed(
    () => new Map(this.organizations().map((o) => [o.id, o.display_name])),
  )

  private organizationName(id: string | undefined): string {
    return id ? (this.organizationNamesById().get(id) ?? id) : ''
  }

  readonly organizationOptions = computed<SelectOption<string>[]>(() => [
    { value: ALL_ORGANIZATIONS, label: 'Toutes les organisations' },
    ...this.organizations().map((o) => ({ value: o.id, label: o.display_name })),
  ])

  readonly filteredRepositories = computed(() => {
    const organizationId = this.selectedOrganizationId()
    return organizationId === ALL_ORGANIZATIONS
      ? this.repositories()
      : this.repositories().filter((r) => r.organization_id === organizationId)
  })

  readonly columns = computed<TableColumn<RepositorySummary>[]>(() => {
    const columns: TableColumn<RepositorySummary>[] = [
      { key: 'name', label: 'Nom' },
      {
        key: 'owner_name',
        label: 'Propriétaire',
        format: (r) => (r.owner_is_personal ? `@${r.owner_name}` : r.owner_name),
      },
    ]
    if (this.showOrganizationControls()) {
      columns.push({
        key: 'organization_id',
        label: 'Organisation',
        format: (r) => this.organizationName(r.organization_id),
      })
    }
    columns.push({ key: 'format', label: 'Format' }, { key: 'repo_type', label: 'Type' })
    return columns
  })
  readonly rowId = (r: RepositorySummary): string => r.id

  // Set once — a later reload() must not snap the filter back and discard the viewer's pick.
  private hasAppliedDefaultOrganizationFilter = false

  ngOnInit(): void {
    this.reload()
  }

  reload(): void {
    this.error.set(null)
    if (this.mode() === 'personal') {
      // No organizations to join against here — every project in this view is the caller's own.
      this.personalRepositoryService.listMyProjects().subscribe({
        next: (repositories) => {
          this.repositories.set(repositories)
          this.loading.set(false)
        },
        error: () => {
          this.loading.set(false)
          this.error.set('Échec du chargement des dépôts.')
        },
      })
      return
    }
    if (!this.isSuperAdmin()) {
      // Can't call /api/organizations (super-admin only) — no forkJoin needed here.
      this.repositoriesService.list().subscribe({
        next: (repositories) => {
          this.repositories.set(repositories)
          this.loading.set(false)
        },
        error: () => {
          this.loading.set(false)
          this.error.set('Échec du chargement des dépôts.')
        },
      })
      return
    }
    forkJoin({
      repositories: this.repositoriesService.list(),
      organizations: this.organizationsService.list(),
    }).subscribe({
      next: ({ repositories, organizations }) => {
        this.repositories.set(repositories)
        this.organizations.set(organizations)
        if (!this.hasAppliedDefaultOrganizationFilter) {
          // Defaults to the public organization, not every organization at once.
          const publicOrganization = organizations.find((o) => o.is_public)
          this.selectedOrganizationId.set(publicOrganization?.id ?? ALL_ORGANIZATIONS)
          this.hasAppliedDefaultOrganizationFilter = true
        }
        this.loading.set(false)
      },
      error: () => {
        this.loading.set(false)
        this.error.set('Échec du chargement des dépôts.')
      },
    })
  }

  onCreated(): void {
    this.showCreateModal.set(false)
    this.reload()
  }

  openDetail(repository: RepositorySummary): void {
    this.router.navigate(['/repositories', repository.id])
  }
}
