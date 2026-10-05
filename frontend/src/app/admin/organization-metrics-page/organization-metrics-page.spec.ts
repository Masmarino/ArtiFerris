import { TestBed } from '@angular/core/testing'
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing'
import { provideHttpClient } from '@angular/common/http'
import { OrganizationMetricsPage } from './organization-metrics-page'
import { adminProviders } from '../infrastructure/admin.providers'

function render(organizationId?: string) {
  TestBed.configureTestingModule({
    imports: [OrganizationMetricsPage],
    providers: [provideHttpClient(), provideHttpClientTesting(), ...adminProviders],
  })
  const fixture = TestBed.createComponent(OrganizationMetricsPage)
  const httpMock = TestBed.inject(HttpTestingController)
  if (organizationId !== undefined) {
    fixture.componentRef.setInput('organizationId', organizationId)
  }
  fixture.detectChanges()
  return { fixture, httpMock }
}

describe('OrganizationMetricsPage', () => {
  it('renders the stats cards in a grid and the usage metrics component', () => {
    const { fixture, httpMock } = render('org-1')
    httpMock
      .expectOne((r) => r.url === '/api/admin/stats' && r.params.get('organization_id') === 'org-1')
      .flush({ total_users: 2, total_repositories: 1, total_active_permissions: 0 })
    httpMock
      .expectOne(
        (r) => r.url === '/api/admin/metrics' && r.params.get('organization_id') === 'org-1',
      )
      .flush([])
    fixture.detectChanges()

    expect(fixture.nativeElement.querySelector('.stats-grid')).not.toBeNull()
    const text = fixture.nativeElement.textContent as string
    expect(text).toContain('Utilisateurs')
    expect(text).toContain('2')
    expect(text).toContain('Dépôts')
    expect(text).toContain("Droits d'accès")
  })

  it('re-fetches stats when organizationId changes to a different organization', () => {
    const { fixture, httpMock } = render('org-1')
    httpMock
      .expectOne((r) => r.params.get('organization_id') === 'org-1' && r.url === '/api/admin/stats')
      .flush({ total_users: 2, total_repositories: 1, total_active_permissions: 0 })
    httpMock.expectOne((r) => r.url === '/api/admin/metrics').flush([])
    fixture.detectChanges()

    fixture.componentRef.setInput('organizationId', 'org-2')
    fixture.detectChanges()
    httpMock
      .expectOne((r) => r.params.get('organization_id') === 'org-2' && r.url === '/api/admin/stats')
      .flush({ total_users: 5, total_repositories: 3, total_active_permissions: 1 })
    httpMock
      .expectOne(
        (r) => r.params.get('organization_id') === 'org-2' && r.url === '/api/admin/metrics',
      )
      .flush([])
    fixture.detectChanges()

    expect(fixture.nativeElement.textContent).toContain('5')
  })

  it('keeps the current organization when a slower stats response for the previous one lands last', () => {
    const { fixture, httpMock } = render('org-a')
    const forA = httpMock.expectOne(
      (r) => r.url === '/api/admin/stats' && r.params.get('organization_id') === 'org-a',
    )
    fixture.componentRef.setInput('organizationId', 'org-b')
    fixture.detectChanges()
    const forB = httpMock.expectOne(
      (r) => r.url === '/api/admin/stats' && r.params.get('organization_id') === 'org-b',
    )

    forB.flush({ total_users: 7, total_repositories: 1, total_active_permissions: 0 })
    forA.flush({ total_users: 99, total_repositories: 1, total_active_permissions: 0 })

    expect(fixture.componentInstance.stats()?.total_users).toBe(7)
  })

  it('shows an error when the stats cannot be loaded', () => {
    const { fixture, httpMock } = render('org-a')

    httpMock
      .expectOne((r) => r.url === '/api/admin/stats')
      .flush(null, { status: 500, statusText: 'boom' })
    fixture.detectChanges()

    expect(fixture.nativeElement.textContent).toContain('Échec du chargement des statistiques.')
  })
})
