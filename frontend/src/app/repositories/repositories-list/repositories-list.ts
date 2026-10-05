import { t } from '../../shared/i18n/translator'
import { TranslocoPipe } from '@jsverse/transloco'
import {
  ChangeDetectionStrategy,
  Component,
  OnInit,
  computed,
  Injector,
  inject,
  input,
  signal,
} from '@angular/core'
import { Router } from '@angular/router'
import { FormsModule } from '@angular/forms'
import { forkJoin } from 'rxjs'
import { Button } from '@masmarino/gabarit/button'
import { EmptyState } from '@masmarino/gabarit/empty-state'
import { Select, SelectOption } from '@masmarino/gabarit/select'
import { Spinner } from '@masmarino/gabarit/spinner'
import { Table, TableColumn } from '@masmarino/gabarit/table'
import { CreateRepositoryModal } from '../create-repository-modal/create-repository-modal'
import { CreateUserProjectModal } from '../create-user-project-modal/create-user-project-modal'
import { RepositoriesService } from '../application/repositories.service'
import { PersonalRepositoryService } from '../application/personal-repository.service'
import { RepositorySummary } from '../domain/repository.entity'
import { OrganizationsService } from '../../admin/application/organizations.service'
import { OrganizationSummary } from '../../admin/domain/organization.entity'
import { MeService } from '../../shell/application/me.service'
import { PageHeading } from '../../shared/page-heading/page-heading'
import { openWhenAsked } from '../../shared/open-when-asked'

/** Not a real organization id: selects the unfiltered view across organizations. */
const ALL_ORGANIZATIONS = 'ALL'

@Component({
  selector: 'app-repositories-list',
  standalone: true,
  imports: [
    PageHeading,
    TranslocoPipe,
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
  private readonly injector = inject(Injector)

  // 'personal' is the caller's own namespace: a single organization, so organization controls are
  // meaningless.
  readonly mode = input<'organization' | 'personal'>('organization')

  // Only a super-admin sees repositories across organizations, so the filter and column are for
  // them.
  readonly isSuperAdmin = computed(() => this.me.isSuperAdmin())

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
    { value: ALL_ORGANIZATIONS, label: t('users.list.allOrganizations') },
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
      { key: 'name', label: t('common.name') },
      {
        key: 'owner_name',
        label: t('repositories.list.columns.owner'),
        format: (r) => (r.owner_is_personal ? `@${r.owner_name}` : r.owner_name),
      },
    ]
    if (this.showOrganizationControls()) {
      columns.push({
        key: 'organization_id',
        label: t('users.list.organization'),
        format: (r) => this.organizationName(r.organization_id),
      })
    }
    columns.push(
      { key: 'format', label: t('common.format') },
      { key: 'repo_type', label: t('common.type') },
    )
    return columns
  })
  readonly rowId = (r: RepositorySummary): string => r.id

  // Set once: a later reload() must not reset the viewer's pick.
  private hasAppliedDefaultOrganizationFilter = false

  ngOnInit(): void {
    this.reload()
    // The quick search's "Nouveau dépôt" / "Nouveau projet": which one depends on the mode, an input.
    openWhenAsked(
      this.mode() === 'personal' ? 'project' : 'repository',
      () => this.showCreateModal.set(true),
      this.injector,
    )
  }

  reload(): void {
    this.error.set(null)
    if (this.mode() === 'personal') {
      // Every project here is the caller's own: no organizations to join.
      this.personalRepositoryService.listMyProjects().subscribe({
        next: (repositories) => {
          this.repositories.set(repositories)
          this.loading.set(false)
        },
        error: () => {
          this.loading.set(false)
          this.error.set(t('repositories.list.errors.loadFailed'))
        },
      })
      return
    }
    if (!this.isSuperAdmin()) {
      // /api/organizations is super-admin only: no forkJoin here.
      this.repositoriesService.list().subscribe({
        next: (repositories) => {
          this.repositories.set(repositories)
          this.loading.set(false)
        },
        error: () => {
          this.loading.set(false)
          this.error.set(t('repositories.list.errors.loadFailed'))
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
          // Defaults to the public organization, not all of them.
          const publicOrganization = organizations.find((o) => o.is_public)
          this.selectedOrganizationId.set(publicOrganization?.id ?? ALL_ORGANIZATIONS)
          this.hasAppliedDefaultOrganizationFilter = true
        }
        this.loading.set(false)
      },
      error: () => {
        this.loading.set(false)
        this.error.set(t('repositories.list.errors.loadFailed'))
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
