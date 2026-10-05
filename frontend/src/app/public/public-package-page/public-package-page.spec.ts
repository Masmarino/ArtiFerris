import { ComponentFixture, TestBed } from '@angular/core/testing'
import { ActivatedRoute, convertToParamMap, provideRouter } from '@angular/router'
import { BehaviorSubject, Subject, of, throwError } from 'rxjs'
import { By } from '@angular/platform-browser'
import { Tab } from '@masmarino/gabarit/tabs'
import { HttpErrorResponse } from '@angular/common/http'
import { AuthService } from '../../auth/application/auth.service'
import { NO_SUGGESTIONS } from '../catalog/testing/no-suggestions'
import { PublicPackagePage } from './public-package-page'
import { RepositoriesService } from '../../repositories/application/repositories.service'
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
    is_public: true,
    my_role: null,
    owner_name: 'alice',
    owner_is_personal: true,
    ...overrides,
  }
}

const NPM_DETAILS = {
  name: 'left-pad',
  versions: [
    {
      version: '1.0.0',
      published_at: '2024-01-01T00:00:00Z',
      size_bytes: 512,
      deprecated: false,
      deprecated_message: null,
      shasum: 'abc',
    },
  ],
  dist_tags: [{ tag: 'latest', version: '1.0.0' }],
  truncated: false,
  readme_html: '<p>Hello <strong>readme</strong></p>',
  registry_url: 'http://localhost:4200/npm/u/alice/my-lib/',
  downloads_7d: 0,
}

const DOCKER_DETAILS = {
  image_name: 'hello',
  image_reference: 'localhost:4200/u/alice/my-image/hello',
  downloads_7d: 0,
  truncated: false,
  tags: [
    {
      tag: 'v1.0.0',
      digest: 'sha256:abc',
      media_type: 'application/vnd.oci.image.manifest.v1+json',
      created_at: '2024-01-01T00:00:00Z',
      size_bytes: 5_242_880,
    },
    {
      tag: 'latest',
      digest: 'sha256:def',
      media_type: 'application/vnd.oci.image.index.v1+json',
      created_at: '2024-02-01T00:00:00Z',
      size_bytes: null,
    },
  ],
}

const DOCKER_PARAMS = { username: '@alice', repoName: 'my-image', format: 'docker', name: 'hello' }

// whenStable() never resolves while a test holds a request open: just let the loader run and re-
// render.
async function settle(fixture: ComponentFixture<unknown>): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve))
  fixture.detectChanges()
}

async function render(
  params: Record<string, string> = {
    username: '@alice',
    repoName: 'my-lib',
    format: 'npm',
    name: 'left-pad',
  },
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
        provide: RepositoriesService,
        useValue: {
          getByOwner: () => of(repo()),
          getByOrg: () => of(repo({ id: 'org-repo-1', owner_is_personal: false })),
          npmPackageDetails: () => of(NPM_DETAILS),
          dockerImageDetails: () => of({ ...DOCKER_DETAILS, tags: [] }),
          npmPackageAudit: () => of([]),
          getDependencyAudit: () => of(null),
          getDockerImageScan: () => of(null),
          ...overrides,
        },
      },
    ],
  })
  const fixture = TestBed.createComponent(PublicPackagePage)
  fixture.detectChanges()
  await settle(fixture)
  return { fixture }
}

describe('PublicPackagePage', () => {
  it('never calls a scan-triggering endpoint, only the read-only results', async () => {
    const scanDependencyTree = vi.fn()
    const scanDockerImage = vi.fn()
    await render(undefined, { scanDependencyTree, scanDockerImage } as never)

    expect(scanDependencyTree).not.toHaveBeenCalled()
    expect(scanDockerImage).not.toHaveBeenCalled()
  })

  it('fetches the npm advisories and dependency audit for the latest version', async () => {
    const npmPackageAudit = vi.fn().mockReturnValue(of([]))
    const getDependencyAudit = vi.fn().mockReturnValue(of(null))
    await render(undefined, { npmPackageAudit, getDependencyAudit })

    expect(npmPackageAudit).toHaveBeenCalledWith('repo-1', 'left-pad')
    expect(getDependencyAudit).toHaveBeenCalledWith('repo-1', 'left-pad', '1.0.0')
  })

  it('shows the Aperçu and Sécurité tabs for an npm package', async () => {
    const { fixture } = await render()

    const labels = fixture.debugElement
      .queryAll(By.directive(Tab))
      .map((tab) => (tab.componentInstance as Tab).label())
    expect(labels).toEqual(['Aperçu', 'Sécurité'])
  })

  it('lists npm registry advisories in the Sécurité tab', async () => {
    const { fixture } = await render(undefined, {
      npmPackageAudit: () =>
        of([
          {
            id: 1,
            url: 'https://example.test/advisory/1',
            title: 'Prototype pollution',
            severity: 'high',
            vulnerable_versions: '<2.0.0',
            cwe: [],
            cvss_score: 7.5,
          },
        ]),
    })

    expect(fixture.nativeElement.textContent).toContain('Prototype pollution')
    expect(fixture.nativeElement.textContent).toContain('high')
  })

  it('shows a clean state when the npm dependency audit found nothing', async () => {
    const { fixture } = await render(undefined, {
      getDependencyAudit: () =>
        of({
          scanned_at: '2024-01-01T00:00:00Z',
          packages_scanned: 12,
          truncated: false,
          findings: [],
        }),
    })

    expect(fixture.nativeElement.textContent).toContain(
      "Aucune vulnérabilité connue dans l'arbre de dépendances.",
    )
  })

  it('lists dependency-audit findings in the Sécurité tab', async () => {
    const { fixture } = await render(undefined, {
      getDependencyAudit: () =>
        of({
          scanned_at: '2024-01-01T00:00:00Z',
          packages_scanned: 12,
          truncated: false,
          findings: [
            {
              dependency_name: 'lodash',
              dependency_version: '4.17.15',
              advisory: {
                id: 2,
                url: 'https://example.test/advisory/2',
                title: 'Command injection',
                severity: 'critical',
                vulnerable_versions: '<4.17.21',
                cwe: [],
                cvss_score: 9.8,
              },
            },
          ],
        }),
    })

    expect(fixture.nativeElement.textContent).toContain('Command injection')
    expect(fixture.nativeElement.textContent).toContain('lodash@4.17.15')
  })

  it('fetches and lists the docker image scan for the preferred tag', async () => {
    const getDockerImageScan = vi.fn().mockReturnValue(
      of({
        scanned_at: '2024-01-01T00:00:00Z',
        vulnerabilities: [
          {
            id: 'CVE-2024-0001',
            package_name: 'openssl',
            installed_version: '1.1.1',
            fixed_version: '1.1.1k',
            severity: 'HIGH',
            title: 'Buffer overflow',
            primary_url: 'https://example.test/CVE-2024-0001',
          },
        ],
      }),
    )
    const { fixture } = await render(DOCKER_PARAMS, {
      dockerImageDetails: () => of(DOCKER_DETAILS),
      getDockerImageScan,
    })

    expect(getDockerImageScan).toHaveBeenCalledWith('repo-1', 'hello', 'latest')
    expect(fixture.nativeElement.textContent).toContain('Buffer overflow')
    expect(fixture.nativeElement.textContent).toContain('openssl@1.1.1')
  })

  it('resolves the owning repository first, then fetches npm details by its id', async () => {
    const getByOwner = vi.fn().mockReturnValue(of(repo()))
    const npmPackageDetails = vi.fn().mockReturnValue(of(NPM_DETAILS))
    await render(undefined, { getByOwner, npmPackageDetails })

    expect(getByOwner).toHaveBeenCalledWith('alice', 'my-lib')
    expect(npmPackageDetails).toHaveBeenCalledWith('repo-1', 'left-pad')
  })

  it('renders dist-tags and versions for an npm package', async () => {
    const { fixture } = await render()

    expect(fixture.nativeElement.textContent).toContain('latest → 1.0.0')
    expect(fixture.nativeElement.textContent).toContain('1.0.0')
  })

  it('shows the weekly downloads of an npm package under its name', async () => {
    const { fixture } = await render(undefined, {
      npmPackageDetails: () => of({ ...NPM_DETAILS, downloads_7d: 1234 }),
    })

    const downloads = fixture.nativeElement.querySelector('.public-package-page__downloads')
    expect(downloads.textContent).toBe('1\u202f234 téléchargements cette semaine')
    expect(downloads.previousElementSibling.tagName).toBe('H1')
  })

  it('shows the weekly downloads of a docker image', async () => {
    const { fixture } = await render(DOCKER_PARAMS, {
      dockerImageDetails: () => of({ ...DOCKER_DETAILS, downloads_7d: 1 }),
    })

    expect(fixture.nativeElement.querySelector('.public-package-page__downloads').textContent).toBe(
      '1 téléchargement cette semaine',
    )
  })

  it('hides the downloads when there were none', async () => {
    const npm = await render()
    expect(npm.fixture.nativeElement.querySelector('.public-package-page__downloads')).toBeNull()

    TestBed.resetTestingModule()
    const docker = await render(DOCKER_PARAMS, { dockerImageDetails: () => of(DOCKER_DETAILS) })
    expect(docker.fixture.nativeElement.querySelector('.public-package-page__downloads')).toBeNull()
  })

  it('warns that only the newest 200 versions are listed when the backend truncated them', async () => {
    const { fixture } = await render(undefined, {
      npmPackageDetails: () => of({ ...NPM_DETAILS, truncated: true }),
    })

    expect(
      fixture.nativeElement.querySelector('[data-testid="truncated-notice"]').textContent,
    ).toContain('Seules les 200 versions les plus récentes sont affichées')
  })

  it('warns that only the newest 100 tags are listed when the backend truncated them', async () => {
    const { fixture } = await render(DOCKER_PARAMS, {
      dockerImageDetails: () => of({ ...DOCKER_DETAILS, truncated: true }),
    })

    expect(
      fixture.nativeElement.querySelector('[data-testid="truncated-notice"]').textContent,
    ).toContain('Seuls les 100 tags les plus récents sont affichés')
  })

  it('shows no truncation notice for a complete list', async () => {
    const npm = await render()
    expect(npm.fixture.nativeElement.querySelector('[data-testid="truncated-notice"]')).toBeNull()

    TestBed.resetTestingModule()
    const docker = await render(DOCKER_PARAMS, { dockerImageDetails: () => of(DOCKER_DETAILS) })
    expect(
      docker.fixture.nativeElement.querySelector('[data-testid="truncated-notice"]'),
    ).toBeNull()
  })

  it('renders tags for a docker image', async () => {
    const { fixture } = await render(DOCKER_PARAMS, {
      dockerImageDetails: () => of(DOCKER_DETAILS),
    })

    const cells = Array.from(fixture.nativeElement.querySelectorAll('tbody td')).map((td) =>
      (td as HTMLElement).textContent!.trim(),
    )
    expect(cells).toContain('v1.0.0')
    expect(cells).toContain('latest')
  })

  it('shows the npm install command from the registry URL, with a copy button', async () => {
    const { fixture } = await render()

    const install = fixture.nativeElement.querySelector('app-copyable-command')
    expect(install.querySelector('pre').textContent).toBe(
      'npm install left-pad --registry http://localhost:4200/npm/u/alice/my-lib/',
    )
    expect(install.querySelector('button').getAttribute('aria-label')).toBe(
      "Copier la commande d'installation de left-pad",
    )
  })

  it('renders the README between the dist-tags and the versions table', async () => {
    const { fixture } = await render()

    const el: HTMLElement = fixture.nativeElement
    const content = el.querySelector('app-readme-view .readme-view__content')!
    expect(content.querySelector('strong')?.textContent).toBe('readme')
    const readme = el.querySelector('app-readme-view')!
    const table = el.querySelector('table')!
    expect(
      el.querySelector('.public-package-page__dist-tags')!.compareDocumentPosition(readme),
    ).toBe(Node.DOCUMENT_POSITION_FOLLOWING)
    expect(readme.compareDocumentPosition(table)).toBe(Node.DOCUMENT_POSITION_FOLLOWING)
  })

  it('shows the empty README state when the package has none', async () => {
    const { fixture } = await render(undefined, {
      npmPackageDetails: () => of({ ...NPM_DETAILS, readme_html: null }),
    })

    const el: HTMLElement = fixture.nativeElement
    expect(el.querySelector('app-readme-view .readme-view__content')).toBeNull()
    expect(el.querySelector('app-readme-view')!.textContent).toContain('Aucun README')
  })

  it('shows the docker pull command with the latest tag, even when it is not first', async () => {
    const { fixture } = await render(DOCKER_PARAMS, {
      dockerImageDetails: () => of(DOCKER_DETAILS),
    })

    expect(fixture.nativeElement.querySelector('app-copyable-command pre').textContent).toBe(
      'docker pull localhost:4200/u/alice/my-image/hello:latest',
    )
  })

  it('uses the first tag when there is no latest', async () => {
    const { fixture } = await render(DOCKER_PARAMS, {
      dockerImageDetails: () =>
        of({ ...DOCKER_DETAILS, tags: DOCKER_DETAILS.tags.filter((t) => t.tag !== 'latest') }),
    })

    expect(fixture.nativeElement.querySelector('app-copyable-command pre').textContent).toBe(
      'docker pull localhost:4200/u/alice/my-image/hello:v1.0.0',
    )
  })

  it('shows the bare image reference when there are no tags', async () => {
    const { fixture } = await render(DOCKER_PARAMS, {
      dockerImageDetails: () => of({ ...DOCKER_DETAILS, tags: [] }),
    })

    expect(fixture.nativeElement.querySelector('app-copyable-command pre').textContent).toBe(
      'docker pull localhost:4200/u/alice/my-image/hello',
    )
  })

  it('adds a size column to the docker tags, with a dash when the size is unknown', async () => {
    const { fixture } = await render(DOCKER_PARAMS, {
      dockerImageDetails: () => of(DOCKER_DETAILS),
    })

    const el: HTMLElement = fixture.nativeElement
    expect(Array.from(el.querySelectorAll('th')).map((th) => th.textContent)).toEqual([
      'Tag',
      'Créé le',
      'Taille',
    ])
    const sizes = Array.from(el.querySelectorAll('tbody tr')).map(
      (row) => row.querySelectorAll('td')[2].textContent,
    )
    expect(sizes).toEqual(['5,0 Mo', '—'])
  })

  it('shows a not-found message when the owning repository cannot be resolved', async () => {
    const { fixture } = await render(undefined, {
      getByOwner: () => throwError(() => new HttpErrorResponse({ status: 404 })),
    })

    expect(fixture.nativeElement.textContent).toContain('introuvable')
  })

  it('renders a loading state before the package request resolves', async () => {
    const { fixture } = await render(undefined, {
      getByOwner: () => new Subject<RepositorySummary>(),
    })

    expect(fixture.nativeElement.textContent).toContain('Chargement')
    expect(fixture.nativeElement.textContent).not.toContain('introuvable')
  })

  it('shows a distinct error message for a server error, not the same "not found" text', async () => {
    const { fixture } = await render(undefined, {
      getByOwner: () => throwError(() => new HttpErrorResponse({ status: 500 })),
    })

    expect(fixture.nativeElement.textContent).not.toContain('Package introuvable.')
    expect(fixture.nativeElement.textContent).not.toContain('introuvable')
  })

  it('still shows "not found" for an actual 404', async () => {
    const { fixture } = await render(undefined, {
      getByOwner: () => throwError(() => new HttpErrorResponse({ status: 404 })),
    })

    expect(fixture.nativeElement.textContent).toContain('Package introuvable.')
  })

  it('renders the package name in an h1', async () => {
    const { fixture } = await render()

    const h1 = fixture.debugElement.query(By.css('h1'))
    expect(h1.nativeElement.textContent).toBe('left-pad')
  })

  it('links back to the public repository page, not the authenticated one', async () => {
    const { fixture } = await render()

    // PublicLayout renders its own "Se connecter" link first: target the back-link by its class.
    const back = fixture.debugElement.query(By.css('.public-package-page__back'))
    expect(back.attributes['href']).toBe('/@alice/my-lib')
  })

  describe('route reuse', () => {
    const first = { username: '@alice', repoName: 'r1', format: 'npm', name: 'x' }
    const second = { username: '@bob', repoName: 'r2', format: 'npm', name: 'y' }

    function reused(overrides: Partial<RepositoriesService>) {
      const paramMap = new BehaviorSubject(convertToParamMap(first))
      TestBed.configureTestingModule({
        providers: [
          provideRouter([]),
          NO_SUGGESTIONS,
          { provide: AuthService, useValue: { isAuthenticated: () => false } },
          { provide: ActivatedRoute, useValue: { paramMap } },
          {
            provide: RepositoriesService,
            useValue: {
              getByOwner: (_owner: string, name: string) => of(repo({ id: `id-${name}` })),
              npmPackageAudit: () => of([]),
              getDependencyAudit: () => of(null),
              getDockerImageScan: () => of(null),
              ...overrides,
            },
          },
        ],
      })
      const fixture = TestBed.createComponent(PublicPackagePage)
      fixture.detectChanges()
      return { fixture, paramMap }
    }

    it('reloads the package when only the route params change', async () => {
      const { fixture, paramMap } = reused({
        npmPackageDetails: (_id: string, name: string) =>
          of({
            ...NPM_DETAILS,
            registry_url: `http://reg/${name}/`,
            readme_html: `<p>${name}</p>`,
          }),
      })
      await settle(fixture)
      expect(fixture.nativeElement.querySelector('app-copyable-command pre').textContent).toBe(
        'npm install x --registry http://reg/x/',
      )

      paramMap.next(convertToParamMap(second))
      fixture.detectChanges()
      await settle(fixture)

      expect(fixture.nativeElement.querySelector('h1').textContent).toBe('y')
      expect(fixture.nativeElement.querySelector('app-copyable-command pre').textContent).toBe(
        'npm install y --registry http://reg/y/',
      )
      expect(fixture.nativeElement.querySelector('app-readme-view').textContent).toContain('y')
    })

    it('ignores a slow response for the package the visitor already left', async () => {
      const slow = new Subject<typeof NPM_DETAILS>()
      const { fixture, paramMap } = reused({
        npmPackageDetails: (_id: string, name: string) =>
          name === 'x' ? slow : of({ ...NPM_DETAILS, registry_url: 'http://reg/y/' }),
      })
      await settle(fixture)

      paramMap.next(convertToParamMap(second))
      fixture.detectChanges()
      await settle(fixture)
      slow.next({ ...NPM_DETAILS, registry_url: 'http://reg/x/' })
      fixture.detectChanges()
      await settle(fixture)

      expect(fixture.nativeElement.querySelector('app-copyable-command pre').textContent).toBe(
        'npm install y --registry http://reg/y/',
      )
    })
  })

  describe('organization route (o/:slug/:repoName/packages/:format/:name)', () => {
    const orgParams = { slug: 'acme', repoName: 'my-lib', format: 'npm', name: 'left-pad' }

    it('resolves the repository through by-org, then fetches details by its id', async () => {
      const getByOwner = vi.fn()
      const getByOrg = vi.fn().mockReturnValue(of(repo({ id: 'org-repo-1' })))
      const npmPackageDetails = vi.fn().mockReturnValue(of(NPM_DETAILS))
      await render(orgParams, { getByOwner, getByOrg, npmPackageDetails })

      expect(getByOrg).toHaveBeenCalledWith('acme', 'my-lib')
      expect(getByOwner).not.toHaveBeenCalled()
      expect(npmPackageDetails).toHaveBeenCalledWith('org-repo-1', 'left-pad')
    })

    it('fetches docker details by the by-org repository id', async () => {
      const dockerImageDetails = vi
        .fn()
        .mockReturnValue(of({ ...DOCKER_DETAILS, image_name: 'team/api', tags: [] }))
      await render({ ...orgParams, format: 'docker', name: 'team/api' }, { dockerImageDetails })

      expect(dockerImageDetails).toHaveBeenCalledWith('org-repo-1', 'team/api')
    })

    it('links back to the organization repository page', async () => {
      const { fixture } = await render(orgParams)

      const back = fixture.debugElement.query(By.css('.public-package-page__back'))
      expect(back.attributes['href']).toBe('/o/acme/my-lib')
    })

    it('shows not-found when the organization repository cannot be resolved', async () => {
      const { fixture } = await render(orgParams, {
        getByOrg: () => throwError(() => new HttpErrorResponse({ status: 404 })),
      })

      expect(fixture.nativeElement.textContent).toContain('Package introuvable.')
    })

    it('renders the package name and versions', async () => {
      const { fixture } = await render(orgParams)

      expect(fixture.debugElement.query(By.css('h1')).nativeElement.textContent).toBe('left-pad')
      expect(fixture.nativeElement.textContent).toContain('latest → 1.0.0')
    })
  })
})
