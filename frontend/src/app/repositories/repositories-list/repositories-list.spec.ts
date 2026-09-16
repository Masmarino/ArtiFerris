import { TestBed } from '@angular/core/testing'
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing'
import { provideHttpClient } from '@angular/common/http'
import { Router, provideRouter } from '@angular/router'
import { By } from '@angular/platform-browser'
import { Select, Table } from '@masmarino/gabarit'
import { RepositoriesList } from './repositories-list'
import { repositoryProviders } from '../infrastructure/repository.providers'
import { organizationsProviders } from '../../admin/infrastructure/organizations.providers'
import { MeService } from '../../shell/application/me.service'

const PUBLIC_ORG = { id: 'org-public', slug: 'public', display_name: 'Public', is_public: true }
const ACME_ORG = { id: 'org-acme', slug: 'acme', display_name: 'Acme Corp', is_public: false }

const PUBLIC_REPO = {
  id: 'r1',
  name: 'my-repo',
  format: 'npm' as const,
  repo_type: 'hosted' as const,
  remote_url: null,
  remote_credentials_set: false,
  group_members: [],
  quota_bytes: null,
  retention_keep_last_n: null,
  my_role: 'admin' as const,
  organization_id: 'org-public',
}
const ACME_REPO = {
  id: 'r2',
  name: 'acme-repo',
  format: 'docker' as const,
  repo_type: 'hosted' as const,
  remote_url: null,
  remote_credentials_set: false,
  group_members: [],
  quota_bytes: null,
  retention_keep_last_n: null,
  my_role: 'admin' as const,
  organization_id: 'org-acme',
}

describe('RepositoriesList', () => {
  afterEach(() => {
    vi.restoreAllMocks()
  })

  function setup(options?: { isSuperAdmin?: boolean }) {
    TestBed.configureTestingModule({
      providers: [
        provideHttpClient(),
        provideHttpClientTesting(),
        provideRouter([]),
        ...repositoryProviders,
        ...organizationsProviders,
        { provide: MeService, useValue: { isSuperAdmin: () => options?.isSuperAdmin ?? true } },
      ],
    })
    const fixture = TestBed.createComponent(RepositoriesList)
    const httpMock = TestBed.inject(HttpTestingController)
    return { fixture, httpMock }
  }

  function flushInitialLoad(
    httpMock: HttpTestingController,
    repositories: unknown[] = [PUBLIC_REPO],
    organizations: unknown[] = [PUBLIC_ORG, ACME_ORG],
  ) {
    httpMock.expectOne('/api/repositories').flush(repositories)
    httpMock.expectOne('/api/organizations').flush(organizations)
  }

  it('loads repositories into the repositories signal on init', () => {
    const { fixture, httpMock } = setup()

    fixture.detectChanges()
    flushInitialLoad(httpMock)

    expect(fixture.componentInstance.repositories().length).toBe(1)
  })

  it('shows a loading state instead of an empty table while the request is in flight', () => {
    const { fixture, httpMock } = setup()

    fixture.detectChanges()

    expect(fixture.nativeElement.textContent).toContain('Chargement…')
    expect(fixture.debugElement.query(By.directive(Table))).toBeNull()

    flushInitialLoad(httpMock)
    fixture.detectChanges()

    expect(fixture.nativeElement.textContent).not.toContain('Chargement…')
    expect(fixture.debugElement.query(By.directive(Table))).toBeTruthy()
  })

  it('navigates to the repository detail page when a table row is clicked', () => {
    const { fixture, httpMock } = setup()
    const router = TestBed.inject(Router)
    vi.spyOn(router, 'navigate')

    fixture.detectChanges()
    flushInitialLoad(httpMock)
    fixture.detectChanges()

    const tableDebugElement = fixture.debugElement.query(By.directive(Table))
    tableDebugElement.triggerEventHandler('rowClick', PUBLIC_REPO)

    expect(router.navigate).toHaveBeenCalledWith(['/repositories', 'r1'])
  })

  it('shows a retryable error instead of hanging on "Chargement…" when the request fails', () => {
    const { fixture, httpMock } = setup()

    fixture.detectChanges()
    httpMock.expectOne('/api/repositories').flush('error', { status: 500, statusText: 'Server Error' })
    httpMock.expectOne('/api/organizations').flush([PUBLIC_ORG])
    fixture.detectChanges()

    expect(fixture.componentInstance.loading()).toBe(false)
    expect(fixture.nativeElement.textContent).not.toContain('Chargement…')
    expect(fixture.componentInstance.error()).not.toBeNull()

    // organizationsService.list() caches — a reload only re-requests repositories.
    fixture.componentInstance.reload()
    httpMock.expectOne('/api/repositories').flush([PUBLIC_REPO])
    fixture.detectChanges()

    expect(fixture.componentInstance.error()).toBeNull()
    expect(fixture.componentInstance.repositories().length).toBe(1)
  })

  it('defaults the organization filter to the public organization', () => {
    const { fixture, httpMock } = setup()

    fixture.detectChanges()
    flushInitialLoad(httpMock, [PUBLIC_REPO, ACME_REPO])

    expect(fixture.componentInstance.selectedOrganizationId()).toBe('org-public')
    expect(fixture.componentInstance.filteredRepositories()).toEqual([PUBLIC_REPO])
  })

  it('shows every repository across organizations when "ALL" is selected', () => {
    const { fixture, httpMock } = setup()

    fixture.detectChanges()
    flushInitialLoad(httpMock, [PUBLIC_REPO, ACME_REPO])
    fixture.componentInstance.selectedOrganizationId.set('ALL')
    fixture.detectChanges()

    expect(fixture.componentInstance.filteredRepositories()).toEqual([PUBLIC_REPO, ACME_REPO])
  })

  it('offers an option per organization plus "Toutes les organisations"', () => {
    const { fixture, httpMock } = setup()

    fixture.detectChanges()
    flushInitialLoad(httpMock)
    fixture.detectChanges()

    expect(fixture.componentInstance.organizationOptions()).toEqual([
      { value: 'ALL', label: 'Toutes les organisations' },
      { value: 'org-public', label: 'Public' },
      { value: 'org-acme', label: 'Acme Corp' },
    ])
  })

  it('renders the organization display name in the table, not the raw id', () => {
    const { fixture, httpMock } = setup()

    fixture.detectChanges()
    flushInitialLoad(httpMock, [PUBLIC_REPO, ACME_REPO])
    fixture.componentInstance.selectedOrganizationId.set('ALL')
    fixture.detectChanges()

    const organizationColumn = fixture.componentInstance
      .columns()
      .find((c) => c.key === 'organization_id')
    expect(organizationColumn?.format?.(ACME_REPO)).toBe('Acme Corp')
  })

  it('keeps the viewer-selected organization filter across a reload triggered by creating a repository', () => {
    const { fixture, httpMock } = setup()

    fixture.detectChanges()
    flushInitialLoad(httpMock, [PUBLIC_REPO, ACME_REPO])
    fixture.componentInstance.selectedOrganizationId.set('ALL')

    // Both services cache their list() — a bare reload() with no intervening mutation
    // replays from cache rather than issuing a second request.
    fixture.componentInstance.reload()

    expect(fixture.componentInstance.selectedOrganizationId()).toBe('ALL')
  })

  it('renders an organization select control', () => {
    const { fixture, httpMock } = setup()

    fixture.detectChanges()
    flushInitialLoad(httpMock)
    fixture.detectChanges()

    expect(fixture.debugElement.query(By.directive(Select))).toBeTruthy()
  })

  describe('as an organization admin', () => {
    it("loads only its own organization's repositories, with a single request and no /api/organizations call", () => {
      const { fixture, httpMock } = setup({ isSuperAdmin: false })

      fixture.detectChanges()
      httpMock.expectOne('/api/repositories').flush([ACME_REPO])
      fixture.detectChanges()

      expect(fixture.componentInstance.repositories()).toEqual([ACME_REPO])
      expect(fixture.componentInstance.filteredRepositories()).toEqual([ACME_REPO])
      httpMock.verify()
    })

    it('hides the organization filter and the organization column', () => {
      const { fixture, httpMock } = setup({ isSuperAdmin: false })

      fixture.detectChanges()
      httpMock.expectOne('/api/repositories').flush([ACME_REPO])
      fixture.detectChanges()

      expect(fixture.debugElement.query(By.directive(Select))).toBeNull()
      expect(fixture.componentInstance.columns().map((c) => c.key)).not.toContain('organization_id')
    })

    it("navigates to the repository detail page on row click, same as a super-admin — an organization admin has the same rights over their own organization's repositories", () => {
      const { fixture, httpMock } = setup({ isSuperAdmin: false })
      const router = TestBed.inject(Router)
      vi.spyOn(router, 'navigate')

      fixture.detectChanges()
      httpMock.expectOne('/api/repositories').flush([ACME_REPO])
      fixture.detectChanges()

      fixture.componentInstance.openDetail(ACME_REPO)

      expect(router.navigate).toHaveBeenCalledWith(['/repositories', 'r2'])
    })

    it('surfaces a retryable error when the scoped request fails', () => {
      const { fixture, httpMock } = setup({ isSuperAdmin: false })

      fixture.detectChanges()
      httpMock.expectOne('/api/repositories').flush('error', { status: 500, statusText: 'Server Error' })
      fixture.detectChanges()

      expect(fixture.componentInstance.loading()).toBe(false)
      expect(fixture.componentInstance.error()).not.toBeNull()
    })
  })
})
