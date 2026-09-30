import { t } from '../../shared/i18n/translator'
import { TranslocoPipe } from '@jsverse/transloco'
import { ChangeDetectionStrategy, Component, OnInit, inject, signal } from '@angular/core'
import { Router } from '@angular/router'
import { Button, EmptyState, Spinner, Table, TableColumn } from '@masmarino/gabarit'
import { CreateOrganizationModal } from '../create-organization-modal/create-organization-modal'
import { OrganizationsService } from '../application/organizations.service'
import { OrganizationSummary } from '../domain/organization.entity'

@Component({
  selector: 'app-organizations-list',
  standalone: true,
  imports: [TranslocoPipe, Table, Button, CreateOrganizationModal, EmptyState, Spinner],
  templateUrl: './organizations-list.html',
  styleUrl: './organizations-list.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class OrganizationsList implements OnInit {
  private readonly organizationsService = inject(OrganizationsService)
  private readonly router = inject(Router)

  readonly organizations = signal<OrganizationSummary[]>([])
  readonly showCreateModal = signal(false)
  readonly loading = signal(true)
  readonly error = signal<string | null>(null)

  readonly columns: TableColumn<OrganizationSummary>[] = [
    { key: 'slug', label: t('admin.organizations.columns.subdomain') },
    { key: 'display_name', label: t('common.name') },
  ]
  readonly rowId = (o: OrganizationSummary): string => o.id

  ngOnInit(): void {
    this.reload()
  }

  reload(): void {
    this.error.set(null)
    this.organizationsService.list({ forceRefresh: true }).subscribe({
      next: (organizations) => {
        this.organizations.set(organizations)
        this.loading.set(false)
      },
      error: () => {
        this.loading.set(false)
        this.error.set(t('admin.organizations.errors.loadFailed'))
      },
    })
  }

  onOrganizationCreated(): void {
    this.showCreateModal.set(false)
    this.reload()
  }

  openDetail(organization: OrganizationSummary): void {
    this.router.navigate(['/admin/organizations', organization.id])
  }
}
