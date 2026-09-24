import { ComponentFixture, TestBed } from '@angular/core/testing'
import { ActivatedRoute, convertToParamMap, provideRouter } from '@angular/router'
import { BehaviorSubject, Subject, of, throwError } from 'rxjs'
import { By } from '@angular/platform-browser'
import { HttpErrorResponse } from '@angular/common/http'
import { AuthService } from '../../auth/application/auth.service'
import { NO_SUGGESTIONS } from '../catalog/testing/no-suggestions'
import { PublicRepositoryPage } from './public-repository-page'
import { RepositoriesService } from '../../repositories/application/repositories.service'
import { PackageTree } from '../../repositories/package-tree/package-tree'
import { RepositorySummary } from '../../repositories/domain/repository.entity'

function repo(overrides: Partial<RepositorySummary> = {}): RepositorySummary {
  return {
    id: 'repo-1',
    name: 'my-lib',
    format: 'npm',
    repo_type: 'hosted',
    remote_url: null,
    remote_credentials_set: false,
    group_members: [],
    quota_bytes: null,
    retention_keep_last_n: null,
    is_public: true,
    my_role: null,
    organization_id: 'org-1',
    owner_name: 'alice',
    owner_is_personal: true,
    ...overrides,
  }
}

// whenStable() never resolves while a test deliberately holds a request open, so just let the
// resource's async loader run and re-render.
async function settle(fixture: ComponentFixture<unknown>): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve))
  fixture.detectChanges()
}

async function render(
  params: Record<string, string>,
  overrides: Partial<RepositoriesService> = {},
) {
  TestBed.configureTestingModule({
    providers: [
      provideRouter([]),
      NO_SUGGESTIONS,
      { provide: AuthService, useValue: { isAuthenticated: () => false } },
      {
        provide: ActivatedRoute,
        useValue: { paramMap: of(convertToParamMap(params)) },
      },
      {
        // PackageTree, rendered once the repository resolves, also injects this service and
        // calls .packages() — it needs a default too, not just getByOwner.
        provide: RepositoriesService,
        useValue: {
          getByOwner: () => of(repo()),
          getByOrg: () => of(repo({ owner_is_personal: false, owner_name: 'Acme' })),
          packages: () => of({ format: 'npm', packages: [] }),
          ...overrides,
        },
      },
    ],
  })
  const fixture = TestBed.createComponent(PublicRepositoryPage)
  fixture.detectChanges()
  await settle(fixture)
  return { fixture }
}

describe('PublicRepositoryPage', () => {
  it('strips the leading @ before resolving the repository', async () => {
    const getByOwner = vi.fn().mockReturnValue(of(repo()))
    await render({ username: '@alice', repoName: 'my-lib' }, { getByOwner })

    expect(getByOwner).toHaveBeenCalledWith('alice', 'my-lib')
  })

  it('renders the repository name, badges, and usage instructions once resolved', async () => {
    const { fixture } = await render({ username: '@alice', repoName: 'my-lib' })

    expect(fixture.nativeElement.textContent).toContain('my-lib')
    expect(fixture.nativeElement.textContent).toContain('npm')
    expect(fixture.nativeElement.textContent).toContain('hosted')
  })

  it('passes a basePath keyed by the raw @username segment to PackageTree', async () => {
    const { fixture } = await render({ username: '@alice', repoName: 'my-lib' })

    const tree = fixture.debugElement.query(By.directive(PackageTree))
    expect(tree.componentInstance.basePath()).toEqual(['/@alice', 'my-lib'])
  })

  it('shows a not-found message for a private or unknown repository, without leaking which', async () => {
    const { fixture } = await render(
      { username: '@nobody', repoName: 'ghost' },
      { getByOwner: () => throwError(() => new HttpErrorResponse({ status: 404 })) },
    )

    expect(fixture.nativeElement.textContent).toContain('introuvable')
    expect(fixture.debugElement.query(By.directive(PackageTree))).toBeFalsy()
  })

  it('renders a loading state before the repository request resolves', async () => {
    const { fixture } = await render(
      { username: '@alice', repoName: 'my-lib' },
      { getByOwner: () => new Subject<RepositorySummary>() },
    )

    expect(fixture.nativeElement.textContent).toContain('Chargement')
    expect(fixture.nativeElement.textContent).not.toContain('introuvable')
    expect(fixture.debugElement.query(By.directive(PackageTree))).toBeFalsy()
  })

  it('shows a distinct error message for a server error, not the same "not found" text', async () => {
    const { fixture } = await render(
      { username: '@alice', repoName: 'my-lib' },
      { getByOwner: () => throwError(() => new HttpErrorResponse({ status: 500 })) },
    )

    expect(fixture.nativeElement.textContent).not.toContain('Dépôt introuvable.')
    expect(fixture.nativeElement.textContent).not.toContain('introuvable')
    expect(fixture.debugElement.query(By.directive(PackageTree))).toBeFalsy()
  })

  it('still shows "not found" for an actual 404', async () => {
    const { fixture } = await render(
      { username: '@alice', repoName: 'my-lib' },
      { getByOwner: () => throwError(() => new HttpErrorResponse({ status: 404 })) },
    )

    expect(fixture.nativeElement.textContent).toContain('Dépôt introuvable.')
  })

  describe('route reuse', () => {
    it('loads the new repository when only the route params change', async () => {
      const paramMap = new BehaviorSubject(
        convertToParamMap({ username: '@alice', repoName: 'r1' }),
      )
      TestBed.configureTestingModule({
        providers: [
          provideRouter([]),
          NO_SUGGESTIONS,
          { provide: AuthService, useValue: { isAuthenticated: () => false } },
          { provide: ActivatedRoute, useValue: { paramMap } },
          {
            provide: RepositoriesService,
            useValue: {
              getByOwner: (_owner: string, name: string) => of(repo({ id: name, name })),
              packages: () => of({ format: 'npm', packages: [] }),
            },
          },
        ],
      })
      const fixture = TestBed.createComponent(PublicRepositoryPage)
      fixture.detectChanges()
      await settle(fixture)
      expect(fixture.nativeElement.querySelector('h1').textContent).toBe('r1')

      paramMap.next(convertToParamMap({ username: '@bob', repoName: 'r2' }))
      fixture.detectChanges()
      await settle(fixture)

      expect(fixture.nativeElement.querySelector('h1').textContent).toBe('r2')
      expect(
        fixture.debugElement.query(By.directive(PackageTree)).componentInstance.basePath(),
      ).toEqual(['/@bob', 'r2'])
    })
  })

  describe('organization route (o/:slug/:repoName)', () => {
    it('resolves the repository through by-org, not by-owner', async () => {
      const getByOwner = vi.fn()
      const getByOrg = vi.fn().mockReturnValue(of(repo({ owner_is_personal: false })))
      await render({ slug: 'acme', repoName: 'my-lib' }, { getByOwner, getByOrg })

      expect(getByOrg).toHaveBeenCalledWith('acme', 'my-lib')
      expect(getByOwner).not.toHaveBeenCalled()
    })

    it('passes a /o/:slug/:repoName basePath to PackageTree', async () => {
      const { fixture } = await render({ slug: 'acme', repoName: 'my-lib' })

      const tree = fixture.debugElement.query(By.directive(PackageTree))
      expect(tree.componentInstance.basePath()).toEqual(['/o', 'acme', 'my-lib'])
    })

    it('renders the repository once resolved', async () => {
      const { fixture } = await render({ slug: 'acme', repoName: 'my-lib' })

      expect(fixture.nativeElement.textContent).toContain('my-lib')
    })

    it('shows not-found for a private or unknown organization repository', async () => {
      const { fixture } = await render(
        { slug: 'acme', repoName: 'ghost' },
        { getByOrg: () => throwError(() => new HttpErrorResponse({ status: 404 })) },
      )

      expect(fixture.nativeElement.textContent).toContain('Dépôt introuvable.')
      expect(fixture.debugElement.query(By.directive(PackageTree))).toBeFalsy()
    })

    it('shows the failure message, not "not found", for a server error', async () => {
      const { fixture } = await render(
        { slug: 'acme', repoName: 'my-lib' },
        { getByOrg: () => throwError(() => new HttpErrorResponse({ status: 500 })) },
      )

      expect(fixture.nativeElement.textContent).toContain('Échec du chargement du dépôt.')
    })
  })
})
