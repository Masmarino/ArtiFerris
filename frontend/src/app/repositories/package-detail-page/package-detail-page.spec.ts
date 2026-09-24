import { TestBed } from '@angular/core/testing'
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing'
import { provideHttpClient } from '@angular/common/http'
import { ActivatedRoute, Router, convertToParamMap, provideRouter } from '@angular/router'
import { BehaviorSubject, of } from 'rxjs'
import { PackageDetailPage } from './package-detail-page'
import { ConfirmService } from '../../shared/confirm.service'
import { PageTitleService } from '../../shell/page-title.service'
import { repositoryProviders } from '../infrastructure/repository.providers'

function render(
  params: { id?: string; format?: string; name?: string } = {},
  confirmAnswer = true,
) {
  const ask = vi.fn().mockResolvedValue(confirmAnswer)
  const paramMap = convertToParamMap({
    id: params.id ?? 'repo-1',
    format: params.format ?? 'npm',
    name: params.name ?? 'left-pad',
  })
  TestBed.configureTestingModule({
    providers: [
      provideHttpClient(),
      provideHttpClientTesting(),
      provideRouter([]),
      ...repositoryProviders,
      { provide: ConfirmService, useValue: { ask } },
      {
        provide: ActivatedRoute,
        useValue: { snapshot: { paramMap }, paramMap: of(paramMap) },
      },
    ],
  })
  const fixture = TestBed.createComponent(PackageDetailPage)
  fixture.detectChanges()
  const httpMock = TestBed.inject(HttpTestingController)
  return { fixture, httpMock, ask }
}

// `render()`'s paramMap is a single-emission `of(...)` — fine for most tests, but
// tests that simulate navigating between packages (Angular reuses this component
// across route param changes) need to push new param maps after creation.
function renderNavigable(params: { id?: string; format?: string; name?: string } = {}) {
  const paramMap$ = new BehaviorSubject(
    convertToParamMap({
      id: params.id ?? 'repo-1',
      format: params.format ?? 'npm',
      name: params.name ?? 'left-pad',
    }),
  )
  TestBed.configureTestingModule({
    providers: [
      provideHttpClient(),
      provideHttpClientTesting(),
      provideRouter([]),
      ...repositoryProviders,
      {
        provide: ActivatedRoute,
        useValue: { snapshot: { paramMap: paramMap$.value }, paramMap: paramMap$ },
      },
    ],
  })
  const fixture = TestBed.createComponent(PackageDetailPage)
  fixture.detectChanges()
  const httpMock = TestBed.inject(HttpTestingController)
  return { fixture, httpMock, paramMap$ }
}

// The delete/rescan buttons only render for a viewer with at least `write`
// on the repository — tests exercising those buttons must flush this
// request with a role that grants it.
function flushRepository(httpMock: HttpTestingController, repositoryId: string, myRole = 'write') {
  httpMock.expectOne(`/api/repositories/${repositoryId}`).flush({
    id: repositoryId,
    name: repositoryId,
    format: 'npm',
    repo_type: 'hosted',
    remote_url: null,
    group_members: [],
    my_role: myRole,
  })
}

function flushAudit(
  httpMock: HttpTestingController,
  repositoryId: string,
  name: string,
  advisories: unknown[] = [],
) {
  httpMock
    .expectOne(`/api/repositories/${repositoryId}/packages/npm/${name}/audit`)
    .flush(advisories)
}

function flushDockerScan(
  httpMock: HttpTestingController,
  repositoryId: string,
  imageName: string,
  tag: string,
  result: object | null = null,
) {
  httpMock
    .expectOne(`/api/repositories/${repositoryId}/packages/docker/${imageName}/tags/${tag}/scan`)
    .flush(result)
}

function flushDependencyAudit(
  httpMock: HttpTestingController,
  repositoryId: string,
  name: string,
  version: string,
  result: object | null = null,
) {
  httpMock
    .expectOne(
      `/api/repositories/${repositoryId}/packages/npm/${name}/versions/${version}/dependency-audit`,
    )
    .flush(result)
}

describe('PackageDetailPage', () => {
  it('sets the shared page title to the package name', () => {
    const { httpMock } = render({ name: 'left-pad' })
    const pageTitle = TestBed.inject(PageTitleService)

    expect(pageTitle.title()).toBe('left-pad')

    httpMock.expectOne('/api/repositories/repo-1/packages/npm/left-pad').flush({
      name: 'left-pad',
      versions: [],
      dist_tags: [],
      readme_html: null,
      registry_url: 'http://localhost:4200/npm/u/alice/repo-1/',
    })
    flushAudit(httpMock, 'repo-1', 'left-pad')
  })

  it('fetches and renders npm package versions and dist-tags', () => {
    const { fixture, httpMock } = render({ format: 'npm', name: 'left-pad' })
    httpMock.expectOne('/api/repositories/repo-1/packages/npm/left-pad').flush({
      name: 'left-pad',
      versions: [
        {
          version: '1.1.0',
          published_at: '2026-01-02T00:00:00Z',
          size_bytes: 2048,
          deprecated: true,
          deprecated_message: 'use v2',
          shasum: 'abc',
        },
      ],
      dist_tags: [{ tag: 'latest', version: '1.1.0' }],
      readme_html: null,
      registry_url: 'http://localhost:4200/npm/u/alice/repo-1/',
    })
    flushAudit(httpMock, 'repo-1', 'left-pad')
    flushDependencyAudit(httpMock, 'repo-1', 'left-pad', '1.1.0')
    fixture.detectChanges()

    const text = fixture.nativeElement.textContent as string
    expect(text).toContain('1.1.0')
    expect(text).toContain('déprécié')
    expect(text).toContain('Ko')
    expect(text).toContain('latest → 1.1.0')
  })

  it('scans the newest published version when the package has no latest tag', () => {
    const { fixture, httpMock } = render({ format: 'npm', name: 'left-pad' })
    const version = (v: string, published_at: string) => ({
      version: v,
      published_at,
      size_bytes: 1,
      deprecated: false,
      deprecated_message: null,
      shasum: 'abc',
    })
    httpMock.expectOne('/api/repositories/repo-1/packages/npm/left-pad').flush({
      name: 'left-pad',
      versions: [
        version('3.4.0-rc.1', '2026-03-01T00:00:00Z'),
        version('2.0.0', '2026-02-01T00:00:00Z'),
        version('0.1.0', '2026-01-01T00:00:00Z'),
      ],
      dist_tags: [{ tag: 'next', version: '3.4.0-rc.1' }],
      readme_html: null,
      registry_url: 'http://localhost:4200/npm/u/alice/repo-1/',
    })
    flushAudit(httpMock, 'repo-1', 'left-pad')
    flushDependencyAudit(httpMock, 'repo-1', 'left-pad', '3.4.0-rc.1')
    fixture.detectChanges()

    expect(fixture.componentInstance.latestVersion()).toBe('3.4.0-rc.1')
  })

  it('deletes an npm version after confirmation, then refetches', async () => {
    const { fixture, httpMock, ask } = render({ format: 'npm', name: 'left-pad' })
    httpMock.expectOne('/api/repositories/repo-1/packages/npm/left-pad').flush({
      name: 'left-pad',
      versions: [
        {
          version: '1.0.0',
          published_at: '2026-01-01T00:00:00Z',
          size_bytes: 1024,
          deprecated: false,
          deprecated_message: null,
          shasum: 'a',
        },
      ],
      dist_tags: [],
      readme_html: null,
      registry_url: 'http://localhost:4200/npm/u/alice/repo-1/',
    })
    flushAudit(httpMock, 'repo-1', 'left-pad')
    flushDependencyAudit(httpMock, 'repo-1', 'left-pad', '1.0.0')
    flushRepository(httpMock, 'repo-1')
    fixture.detectChanges()
    const router = TestBed.inject(Router)
    vi.spyOn(router, 'navigate').mockResolvedValue(true)

    const button: HTMLButtonElement = fixture.nativeElement.querySelector(
      '[data-testid="delete-version-1.0.0"] button',
    )
    button.click()
    await fixture.whenStable()

    expect(ask).toHaveBeenCalledWith(
      expect.objectContaining({ heading: 'Supprimer la version', danger: true }),
    )
    httpMock.expectOne('/api/repositories/repo-1/packages/npm/left-pad/versions/1.0.0').flush(null)

    // The delete cascaded (last version gone) — the refetch 404s and the page navigates back.
    httpMock
      .expectOne('/api/repositories/repo-1/packages/npm/left-pad')
      .flush(null, { status: 404, statusText: 'Not Found' })
  })

  it('does not delete when the confirmation is cancelled', async () => {
    const { fixture, httpMock, ask } = render({ format: 'npm', name: 'left-pad' }, false)
    httpMock.expectOne('/api/repositories/repo-1/packages/npm/left-pad').flush({
      name: 'left-pad',
      versions: [
        {
          version: '1.0.0',
          published_at: '2026-01-01T00:00:00Z',
          size_bytes: 1024,
          deprecated: false,
          deprecated_message: null,
          shasum: 'a',
        },
      ],
      dist_tags: [],
      readme_html: null,
      registry_url: 'http://localhost:4200/npm/u/alice/repo-1/',
    })
    flushAudit(httpMock, 'repo-1', 'left-pad')
    flushDependencyAudit(httpMock, 'repo-1', 'left-pad', '1.0.0')
    flushRepository(httpMock, 'repo-1')
    fixture.detectChanges()

    const button: HTMLButtonElement = fixture.nativeElement.querySelector(
      '[data-testid="delete-version-1.0.0"] button',
    )
    button.click()
    await fixture.whenStable()

    expect(ask).toHaveBeenCalled()
    httpMock.expectNone('/api/repositories/repo-1/packages/npm/left-pad/versions/1.0.0')
  })

  it('deletes the whole npm package and navigates back to the repository', async () => {
    const { fixture, httpMock, ask } = render({ id: 'repo-1', format: 'npm', name: 'left-pad' })
    httpMock.expectOne('/api/repositories/repo-1/packages/npm/left-pad').flush({
      name: 'left-pad',
      versions: [
        {
          version: '1.0.0',
          published_at: '2026-01-01T00:00:00Z',
          size_bytes: 1024,
          deprecated: false,
          deprecated_message: null,
          shasum: 'a',
        },
      ],
      dist_tags: [],
      readme_html: null,
      registry_url: 'http://localhost:4200/npm/u/alice/repo-1/',
    })
    flushAudit(httpMock, 'repo-1', 'left-pad')
    flushDependencyAudit(httpMock, 'repo-1', 'left-pad', '1.0.0')
    flushRepository(httpMock, 'repo-1')
    fixture.detectChanges()
    const router = TestBed.inject(Router)
    const navigate = vi.spyOn(router, 'navigate').mockResolvedValue(true)

    const buttons: HTMLButtonElement[] = Array.from(
      fixture.nativeElement.querySelectorAll('gbt-button button'),
    )
    const wholeDeleteButton = buttons.find((b) =>
      b.textContent?.includes('Supprimer tout le package'),
    )!
    wholeDeleteButton.click()
    await fixture.whenStable()

    expect(ask).toHaveBeenCalledWith(
      expect.objectContaining({ heading: 'Supprimer le package', typeToConfirm: 'left-pad' }),
    )
    httpMock.expectOne('/api/repositories/repo-1/packages/npm/left-pad').flush(null)
    expect(navigate).toHaveBeenCalledWith(['/repositories', 'repo-1'])
  })

  it('fetches and renders docker tags with a shortened digest', () => {
    const { fixture, httpMock } = render({ format: 'docker', name: 'my-app' })
    httpMock.expectOne('/api/repositories/repo-1/packages/docker/my-app').flush({
      image_name: 'my-app',
      image_reference: 'localhost:4200/u/alice/repo-1/my-app',
      tags: [
        {
          tag: 'latest',
          digest: 'sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcd',
          media_type: 'application/vnd.docker.distribution.manifest.v2+json',
          created_at: '2026-01-01T00:00:00Z',
          size_bytes: null,
        },
      ],
    })
    flushDockerScan(httpMock, 'repo-1', 'my-app', 'latest')
    fixture.detectChanges()

    const text = fixture.nativeElement.textContent as string
    expect(text).toContain('latest')
    expect(text).toContain('sha256:0123456789ab…')
    expect(text).not.toContain('0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcd')
  })

  describe('truncated lists', () => {
    const versionBody = (truncated: boolean) => ({
      name: 'left-pad',
      versions: [],
      truncated,
      dist_tags: [],
      readme_html: null,
      registry_url: 'http://localhost:4200/npm/u/alice/repo-1/',
      downloads_7d: 0,
    })
    const tagBody = (truncated: boolean) => ({
      image_name: 'my-app',
      image_reference: 'localhost:4200/u/alice/repo-1/my-app',
      truncated,
      downloads_7d: 0,
      tags: [
        {
          tag: 'latest',
          digest: 'sha256:0123456789abcdef',
          media_type: 'application/vnd.docker.distribution.manifest.v2+json',
          created_at: '2026-01-01T00:00:00Z',
          size_bytes: null,
        },
      ],
    })
    const notice = (fixture: { nativeElement: HTMLElement }) =>
      fixture.nativeElement.querySelector('[data-testid="truncated-notice"]')

    it('warns when only the newest 200 npm versions are listed', () => {
      const { fixture, httpMock } = render({ format: 'npm', name: 'left-pad' })
      httpMock.expectOne('/api/repositories/repo-1/packages/npm/left-pad').flush(versionBody(true))
      flushAudit(httpMock, 'repo-1', 'left-pad')
      fixture.detectChanges()

      expect(notice(fixture)!.textContent).toContain(
        'Seules les 200 versions les plus récentes sont affichées',
      )
    })

    it('warns when only the newest 100 docker tags are listed', () => {
      const { fixture, httpMock } = render({ format: 'docker', name: 'my-app' })
      httpMock.expectOne('/api/repositories/repo-1/packages/docker/my-app').flush(tagBody(true))
      flushDockerScan(httpMock, 'repo-1', 'my-app', 'latest')
      fixture.detectChanges()

      expect(notice(fixture)!.textContent).toContain(
        'Seuls les 100 tags les plus récents sont affichés',
      )
    })

    it('shows nothing when the list is complete', () => {
      const { fixture, httpMock } = render({ format: 'npm', name: 'left-pad' })
      httpMock.expectOne('/api/repositories/repo-1/packages/npm/left-pad').flush(versionBody(false))
      flushAudit(httpMock, 'repo-1', 'left-pad')
      fixture.detectChanges()

      expect(notice(fixture)).toBeNull()
    })
  })

  describe('weekly downloads', () => {
    const NPM_BODY = {
      name: 'left-pad',
      versions: [],
      dist_tags: [],
      readme_html: null,
      registry_url: 'http://localhost:4200/npm/u/alice/repo-1/',
    }
    const DOCKER_BODY = {
      image_name: 'my-app',
      image_reference: 'localhost:4200/u/alice/repo-1/my-app',
      tags: [
        {
          tag: 'latest',
          digest: 'sha256:0123456789abcdef',
          media_type: 'application/vnd.docker.distribution.manifest.v2+json',
          created_at: '2026-01-01T00:00:00Z',
          size_bytes: null,
        },
      ],
    }

    function downloadsOf(fixture: { nativeElement: HTMLElement }) {
      return (
        Array.from(fixture.nativeElement.querySelectorAll('p')).find((p) =>
          (p as HTMLElement).textContent!.includes('téléchargement'),
        ) ?? null
      )
    }

    it('shows them for an npm package', () => {
      const { fixture, httpMock } = render({ format: 'npm', name: 'left-pad' })
      httpMock
        .expectOne('/api/repositories/repo-1/packages/npm/left-pad')
        .flush({ ...NPM_BODY, downloads_7d: 1234 })
      flushAudit(httpMock, 'repo-1', 'left-pad')
      fixture.detectChanges()

      expect(downloadsOf(fixture)!.textContent).toBe('1\u202f234 téléchargements cette semaine')
    })

    it('shows them for a docker image', () => {
      const { fixture, httpMock } = render({ format: 'docker', name: 'my-app' })
      httpMock
        .expectOne('/api/repositories/repo-1/packages/docker/my-app')
        .flush({ ...DOCKER_BODY, downloads_7d: 1 })
      flushDockerScan(httpMock, 'repo-1', 'my-app', 'latest')
      fixture.detectChanges()

      expect(downloadsOf(fixture)!.textContent).toBe('1 téléchargement cette semaine')
    })

    it('hides them at zero', () => {
      const { fixture, httpMock } = render({ format: 'npm', name: 'left-pad' })
      httpMock
        .expectOne('/api/repositories/repo-1/packages/npm/left-pad')
        .flush({ ...NPM_BODY, downloads_7d: 0 })
      flushAudit(httpMock, 'repo-1', 'left-pad')
      fixture.detectChanges()

      expect(downloadsOf(fixture)).toBeNull()
    })
  })

  it('deletes a docker tag after confirmation and navigates back once no tags remain', async () => {
    const { fixture, httpMock, ask } = render({ id: 'repo-1', format: 'docker', name: 'my-app' })
    httpMock.expectOne('/api/repositories/repo-1/packages/docker/my-app').flush({
      image_name: 'my-app',
      image_reference: 'localhost:4200/u/alice/repo-1/my-app',
      tags: [
        {
          tag: 'latest',
          digest: 'sha256:aaaa',
          media_type: 'application/vnd.docker.distribution.manifest.v2+json',
          created_at: '2026-01-01T00:00:00Z',
          size_bytes: null,
        },
      ],
    })
    flushDockerScan(httpMock, 'repo-1', 'my-app', 'latest')
    flushRepository(httpMock, 'repo-1')
    fixture.detectChanges()
    const router = TestBed.inject(Router)
    const navigate = vi.spyOn(router, 'navigate').mockResolvedValue(true)

    const button: HTMLButtonElement = fixture.nativeElement.querySelector(
      '[data-testid="delete-tag-latest"] button',
    )
    button.click()
    await fixture.whenStable()

    expect(ask).toHaveBeenCalledWith(
      expect.objectContaining({ heading: 'Supprimer le tag', danger: true }),
    )
    httpMock.expectOne('/api/repositories/repo-1/packages/docker/my-app/tags/latest').flush(null)
    httpMock.expectOne('/api/repositories/repo-1/packages/docker/my-app').flush({
      image_name: 'my-app',
      image_reference: 'localhost:4200/u/alice/repo-1/my-app',
      tags: [],
    })

    expect(navigate).toHaveBeenCalledWith(['/repositories', 'repo-1'])
  })

  it('links back to the owning repository', () => {
    const { fixture, httpMock } = render({ id: 'repo-1', format: 'npm', name: 'left-pad' })
    const link: HTMLAnchorElement = fixture.nativeElement.querySelector(
      '.package-detail-page__back',
    )

    expect(link.getAttribute('href')).toBe('/repositories/repo-1')

    httpMock.expectOne('/api/repositories/repo-1/packages/npm/left-pad').flush({
      name: 'left-pad',
      versions: [],
      dist_tags: [],
      readme_html: null,
      registry_url: 'http://localhost:4200/npm/u/alice/repo-1/',
    })
    flushAudit(httpMock, 'repo-1', 'left-pad')
  })

  describe('security audit', () => {
    function renderWithDetails() {
      const { fixture, httpMock } = render({ format: 'npm', name: 'left-pad' })
      httpMock.expectOne('/api/repositories/repo-1/packages/npm/left-pad').flush({
        name: 'left-pad',
        versions: [
          {
            version: '1.0.0',
            published_at: '2026-01-01T00:00:00Z',
            size_bytes: 1024,
            deprecated: false,
            deprecated_message: null,
            shasum: 'a',
          },
        ],
        dist_tags: [],
        readme_html: null,
        registry_url: 'http://localhost:4200/npm/u/alice/repo-1/',
      })
      flushDependencyAudit(httpMock, 'repo-1', 'left-pad', '1.0.0')
      flushRepository(httpMock, 'repo-1')
      return { fixture, httpMock }
    }

    it('shows a loading message while the audit request is pending', () => {
      const { fixture } = renderWithDetails()
      fixture.detectChanges()

      expect(fixture.nativeElement.textContent).toContain('Vérification des vulnérabilités')
    })

    it('shows a reassuring message when no advisories are found', () => {
      const { fixture, httpMock } = renderWithDetails()
      flushAudit(httpMock, 'repo-1', 'left-pad', [])
      fixture.detectChanges()

      expect(fixture.nativeElement.textContent).toContain('Aucune vulnérabilité connue')
    })

    it('lists advisories with their severity and vulnerable version range', () => {
      const { fixture, httpMock } = renderWithDetails()
      flushAudit(httpMock, 'repo-1', 'left-pad', [
        {
          id: 123,
          url: 'https://github.com/advisories/GHSA-xxxx',
          title: 'Prototype Pollution',
          severity: 'critical',
          vulnerable_versions: '<0.2.4',
          cwe: ['CWE-1321'],
          cvss_score: 9.8,
        },
      ])
      fixture.detectChanges()

      const text = fixture.nativeElement.textContent as string
      expect(text).toContain('Prototype Pollution')
      expect(text).toContain('critical')
      expect(text).toContain('<0.2.4')
      expect(text).toContain('CVSS 9.8')
      const link: HTMLAnchorElement = fixture.nativeElement.querySelector(
        '.package-detail__advisory-body a',
      )
      expect(link.getAttribute('href')).toBe('https://github.com/advisories/GHSA-xxxx')
      expect(link.getAttribute('target')).toBe('_blank')
    })

    it('shows a soft failure message without breaking the rest of the page when the audit request errors', () => {
      const { fixture, httpMock } = renderWithDetails()
      httpMock
        .expectOne('/api/repositories/repo-1/packages/npm/left-pad/audit')
        .flush(null, { status: 502, statusText: 'Bad Gateway' })
      fixture.detectChanges()

      const text = fixture.nativeElement.textContent as string
      expect(text).toContain('Impossible de vérifier les vulnérabilités')
      expect(text).toContain('1.0.0')
    })

    it.each([
      [429, 'Trop de demandes, réessayez dans un instant'],
      [503, 'Service momentanément occupé'],
    ])('says so when the audit answers a %i, instead of blaming npm', (status, message) => {
      const { fixture, httpMock } = renderWithDetails()
      httpMock
        .expectOne('/api/repositories/repo-1/packages/npm/left-pad/audit')
        .flush({ error: 'busy' }, { status, statusText: 'x' })
      fixture.detectChanges()

      const text = fixture.nativeElement.textContent as string
      expect(text).toContain(message)
      expect(text).not.toContain("base d'alertes npm injoignable")
    })

    it('goes back to the generic message when a retry fails for another reason', () => {
      const { fixture, httpMock } = renderWithDetails()
      const audit = '/api/repositories/repo-1/packages/npm/left-pad/audit'
      httpMock.expectOne(audit).flush({}, { status: 429, statusText: 'x' })
      fixture.detectChanges()
      const rescan = (
        Array.from(
          fixture.nativeElement.querySelectorAll('gbt-button button'),
        ) as HTMLButtonElement[]
      ).find((b) => b.textContent?.includes('Relancer'))
      rescan?.click()
      httpMock.expectOne(audit).flush({}, { status: 502, statusText: 'x' })
      fixture.detectChanges()

      const text = fixture.nativeElement.textContent as string
      expect(text).toContain('Impossible de vérifier les vulnérabilités')
      expect(text).not.toContain('Trop de demandes')
    })

    it('re-fetches the audit when the rescan button is clicked', () => {
      const { fixture, httpMock } = renderWithDetails()
      flushAudit(httpMock, 'repo-1', 'left-pad', [])
      fixture.detectChanges()

      const buttons: HTMLButtonElement[] = Array.from(
        fixture.nativeElement.querySelectorAll('gbt-button button'),
      )
      const rescanButton = buttons.find((b) => b.textContent?.includes('Relancer le scan'))!
      rescanButton.click()
      fixture.detectChanges()

      expect(fixture.nativeElement.textContent).toContain('Vérification des vulnérabilités')
      flushAudit(httpMock, 'repo-1', 'left-pad', [
        {
          id: 123,
          url: 'https://github.com/advisories/GHSA-xxxx',
          title: 'Prototype Pollution',
          severity: 'critical',
          vulnerable_versions: '<0.2.4',
          cwe: ['CWE-1321'],
          cvss_score: 9.8,
        },
      ])
      fixture.detectChanges()

      expect(fixture.nativeElement.textContent).toContain('Prototype Pollution')
    })
  })

  describe('dependency audit', () => {
    function renderWithDetails() {
      const { fixture, httpMock } = render({ format: 'npm', name: 'left-pad' })
      httpMock.expectOne('/api/repositories/repo-1/packages/npm/left-pad').flush({
        name: 'left-pad',
        versions: [
          {
            version: '1.0.0',
            published_at: '2026-01-01T00:00:00Z',
            size_bytes: 1024,
            deprecated: false,
            deprecated_message: null,
            shasum: 'a',
          },
        ],
        dist_tags: [],
        readme_html: '<p>readme</p>',
        registry_url: 'http://localhost:4200/npm/u/alice/repo-1/',
      })
      flushAudit(httpMock, 'repo-1', 'left-pad', [])
      flushRepository(httpMock, 'repo-1')
      return { fixture, httpMock }
    }

    it('prompts to run a scan when none has been recorded yet', () => {
      const { fixture, httpMock } = renderWithDetails()
      flushDependencyAudit(httpMock, 'repo-1', 'left-pad', '1.0.0', null)
      fixture.detectChanges()

      expect(fixture.nativeElement.querySelector('.gbt-empty-state__heading').textContent).toBe(
        'Aucune analyse',
      )
    })

    it('shows the last persisted scan result including findings', () => {
      const { fixture, httpMock } = renderWithDetails()
      flushDependencyAudit(httpMock, 'repo-1', 'left-pad', '1.0.0', {
        scanned_at: '2026-01-05T00:00:00Z',
        packages_scanned: 42,
        truncated: false,
        findings: [
          {
            dependency_name: 'minimist',
            dependency_version: '0.0.8',
            advisory: {
              id: 1097677,
              url: 'https://github.com/advisories/GHSA-xvch-5gv4-984h',
              title: 'Prototype Pollution in minimist',
              severity: 'critical',
              vulnerable_versions: '<0.2.4',
              cwe: ['CWE-1321'],
              cvss_score: 9.8,
            },
          },
        ],
      })
      fixture.detectChanges()

      const text = fixture.nativeElement.textContent as string
      expect(text).toContain('42')
      expect(text).toContain('Prototype Pollution in minimist')
      expect(text).toContain('minimist@0.0.8')
    })

    it('shows a truncation notice when the scan hit its cap', () => {
      const { fixture, httpMock } = renderWithDetails()
      flushDependencyAudit(httpMock, 'repo-1', 'left-pad', '1.0.0', {
        scanned_at: '2026-01-05T00:00:00Z',
        packages_scanned: 500,
        truncated: true,
        findings: [],
      })
      fixture.detectChanges()

      expect(fixture.nativeElement.textContent).toContain('analyse partielle')
    })

    it('says the server is busy when loading the last scan answers a 503', () => {
      const { fixture, httpMock } = renderWithDetails()
      httpMock
        .expectOne('/api/repositories/repo-1/packages/npm/left-pad/versions/1.0.0/dependency-audit')
        .flush({ error: 'busy' }, { status: 503, statusText: 'Service Unavailable' })
      fixture.detectChanges()

      expect(fixture.nativeElement.textContent).toContain('Service momentanément occupé')
    })

    it('tells the user to wait when starting a scan answers a 429', () => {
      const { fixture, httpMock } = renderWithDetails()
      flushDependencyAudit(httpMock, 'repo-1', 'left-pad', '1.0.0', null)
      fixture.detectChanges()
      const buttons: HTMLButtonElement[] = Array.from(
        fixture.nativeElement.querySelectorAll('gbt-button button'),
      )
      buttons.find((b) => b.textContent?.includes('Lancer un scan approfondi'))!.click()

      httpMock
        .expectOne('/api/repositories/repo-1/packages/npm/left-pad/versions/1.0.0/dependency-audit')
        .flush({ error: 'too many' }, { status: 429, statusText: 'Too Many Requests' })
      fixture.detectChanges()

      expect(fixture.nativeElement.textContent).toContain(
        'Trop de demandes, réessayez dans un instant',
      )
    })

    it('triggers a fresh scan and shows the new result when the button is clicked', () => {
      const { fixture, httpMock } = renderWithDetails()
      flushDependencyAudit(httpMock, 'repo-1', 'left-pad', '1.0.0', null)
      fixture.detectChanges()

      const buttons: HTMLButtonElement[] = Array.from(
        fixture.nativeElement.querySelectorAll('gbt-button button'),
      )
      const scanButton = buttons.find((b) => b.textContent?.includes('Lancer un scan approfondi'))!
      scanButton.click()
      fixture.detectChanges()

      const req = httpMock.expectOne(
        '/api/repositories/repo-1/packages/npm/left-pad/versions/1.0.0/dependency-audit',
      )
      expect(req.request.method).toBe('POST')
      req.flush({
        scanned_at: '2026-01-06T00:00:00Z',
        packages_scanned: 3,
        truncated: false,
        findings: [],
      })
      fixture.detectChanges()

      const text = fixture.nativeElement.textContent as string
      expect(text).toContain("Aucune vulnérabilité connue dans l'arbre de dépendances")
      expect(text).toContain('3')
    })
  })

  describe('docker image scan', () => {
    function renderWithDetails() {
      const { fixture, httpMock } = render({ format: 'docker', name: 'my-app' })
      httpMock.expectOne('/api/repositories/repo-1/packages/docker/my-app').flush({
        image_name: 'my-app',
        image_reference: 'localhost:4200/u/alice/repo-1/my-app',
        tags: [
          {
            tag: 'latest',
            digest: 'sha256:aaaa',
            media_type: 'application/vnd.docker.distribution.manifest.v2+json',
            created_at: '2026-01-01T00:00:00Z',
            size_bytes: null,
          },
        ],
      })
      flushRepository(httpMock, 'repo-1')
      return { fixture, httpMock }
    }

    it('prompts to run a scan when none has been recorded yet', () => {
      const { fixture, httpMock } = renderWithDetails()
      flushDockerScan(httpMock, 'repo-1', 'my-app', 'latest', null)
      fixture.detectChanges()

      expect(fixture.nativeElement.querySelector('.gbt-empty-state__heading').textContent).toBe(
        'Aucun scan',
      )
    })

    it('shows the last persisted scan result including vulnerabilities', () => {
      const { fixture, httpMock } = renderWithDetails()
      flushDockerScan(httpMock, 'repo-1', 'my-app', 'latest', {
        scanned_at: '2026-01-05T00:00:00Z',
        vulnerabilities: [
          {
            id: 'CVE-2022-4450',
            package_name: 'libcrypto1.1',
            installed_version: '1.1.1n-r0',
            fixed_version: '1.1.1t-r0',
            severity: 'HIGH',
            title: 'openssl: double free after calling PEM_read_bio_ex',
            primary_url: 'https://avd.aquasec.com/nvd/cve-2022-4450',
          },
        ],
      })
      fixture.detectChanges()

      const text = fixture.nativeElement.textContent as string
      expect(text).toContain('openssl: double free after calling PEM_read_bio_ex')
      expect(text).toContain('libcrypto1.1@1.1.1n-r0')
      expect(text).toContain('1.1.1t-r0')
      expect(text).toContain('HIGH')
    })

    it('shows a reassuring message when no vulnerabilities are found', () => {
      const { fixture, httpMock } = renderWithDetails()
      flushDockerScan(httpMock, 'repo-1', 'my-app', 'latest', {
        scanned_at: '2026-01-05T00:00:00Z',
        vulnerabilities: [],
      })
      fixture.detectChanges()

      expect(fixture.nativeElement.textContent).toContain(
        'Aucune vulnérabilité connue dans cette image',
      )
    })

    it('triggers a fresh scan and shows the new result when the button is clicked', () => {
      const { fixture, httpMock } = renderWithDetails()
      flushDockerScan(httpMock, 'repo-1', 'my-app', 'latest', null)
      fixture.detectChanges()

      const buttons: HTMLButtonElement[] = Array.from(
        fixture.nativeElement.querySelectorAll('gbt-button button'),
      )
      const scanButton = buttons.find((b) => b.textContent?.includes('Lancer un scan'))!
      scanButton.click()
      fixture.detectChanges()

      const req = httpMock.expectOne(
        '/api/repositories/repo-1/packages/docker/my-app/tags/latest/scan',
      )
      expect(req.request.method).toBe('POST')
      req.flush({
        scanned_at: '2026-01-06T00:00:00Z',
        vulnerabilities: [],
      })
      fixture.detectChanges()

      expect(fixture.nativeElement.textContent).toContain(
        'Aucune vulnérabilité connue dans cette image',
      )
    })

    it('shows a soft failure message without breaking the rest of the page when the scan request errors', () => {
      const { fixture, httpMock } = renderWithDetails()
      httpMock
        .expectOne('/api/repositories/repo-1/packages/docker/my-app/tags/latest/scan')
        .flush(null, { status: 502, statusText: 'Bad Gateway' })
      fixture.detectChanges()

      const text = fixture.nativeElement.textContent as string
      expect(text).toContain("Impossible d'effectuer le scan")
      expect(text).toContain('latest')
    })
  })

  describe('docker vulnerability sorting, pagination and filtering', () => {
    function renderWithDetails() {
      const { fixture, httpMock } = render({ format: 'docker', name: 'my-app' })
      httpMock.expectOne('/api/repositories/repo-1/packages/docker/my-app').flush({
        image_name: 'my-app',
        image_reference: 'localhost:4200/u/alice/repo-1/my-app',
        tags: [
          {
            tag: 'latest',
            digest: 'sha256:aaaa',
            media_type: 'application/vnd.docker.distribution.manifest.v2+json',
            created_at: '2026-01-01T00:00:00Z',
            size_bytes: null,
          },
        ],
      })
      flushRepository(httpMock, 'repo-1')
      return { fixture, httpMock }
    }

    function vuln(id: string, severity: string) {
      return {
        id,
        package_name: 'pkg',
        installed_version: '1.0.0',
        fixed_version: null,
        severity,
        title: id,
        primary_url: null,
      }
    }

    it('sorts vulnerabilities from most to least critical regardless of scan order', () => {
      const { fixture, httpMock } = renderWithDetails()
      flushDockerScan(httpMock, 'repo-1', 'my-app', 'latest', {
        scanned_at: '2026-01-05T00:00:00Z',
        vulnerabilities: [
          vuln('v-low', 'LOW'),
          vuln('v-critical', 'CRITICAL'),
          vuln('v-high', 'HIGH'),
        ],
      })
      fixture.detectChanges()

      const severityElements: Element[] = Array.from(
        fixture.nativeElement.querySelectorAll('.package-detail__severity'),
      )
      const severities = severityElements.map((el) => el.textContent?.trim())
      expect(severities).toEqual(['CRITICAL', 'HIGH', 'LOW'])
    })

    it('paginates the vulnerability list at 20 per page', () => {
      const { fixture, httpMock } = renderWithDetails()
      const vulnerabilities = Array.from({ length: 25 }, (_, i) => vuln(`v-${i}`, 'LOW'))
      flushDockerScan(httpMock, 'repo-1', 'my-app', 'latest', {
        scanned_at: '2026-01-05T00:00:00Z',
        vulnerabilities,
      })
      fixture.detectChanges()

      expect(fixture.nativeElement.querySelectorAll('.package-detail__advisory').length).toBe(20)
      expect(fixture.nativeElement.textContent).toContain('Page 1 sur 2')

      const buttons: HTMLButtonElement[] = Array.from(
        fixture.nativeElement.querySelectorAll('gbt-button button'),
      )
      buttons.find((b) => b.textContent?.includes('Suivant'))!.click()
      fixture.detectChanges()

      expect(fixture.nativeElement.querySelectorAll('.package-detail__advisory').length).toBe(5)
      expect(fixture.nativeElement.textContent).toContain('Page 2 sur 2')
    })

    it('filtering by severity narrows the list and resets to page 1', () => {
      const { fixture, httpMock } = renderWithDetails()
      const vulnerabilities = [
        ...Array.from({ length: 25 }, (_, i) => vuln(`v-low-${i}`, 'LOW')),
        vuln('v-critical', 'CRITICAL'),
      ]
      flushDockerScan(httpMock, 'repo-1', 'my-app', 'latest', {
        scanned_at: '2026-01-05T00:00:00Z',
        vulnerabilities,
      })
      fixture.detectChanges()

      const pageButtons: HTMLButtonElement[] = Array.from(
        fixture.nativeElement.querySelectorAll('gbt-button button'),
      )
      pageButtons.find((b) => b.textContent?.includes('Suivant'))!.click()
      fixture.detectChanges()
      expect(fixture.nativeElement.textContent).toContain('Page 2 sur 2')

      const filterTrigger: HTMLButtonElement =
        fixture.nativeElement.querySelector('[role="combobox"]')
      filterTrigger.click()
      fixture.detectChanges()
      const options: HTMLButtonElement[] = Array.from(
        fixture.nativeElement.querySelectorAll('[role="option"]'),
      )
      options.find((o) => o.textContent?.includes('Critique'))!.click()
      fixture.detectChanges()

      expect(fixture.nativeElement.querySelectorAll('.package-detail__advisory').length).toBe(1)
      expect(fixture.nativeElement.textContent).not.toContain('Page 2')
    })

    it('shows a no-match message when the filter matches nothing', () => {
      const { fixture, httpMock } = renderWithDetails()
      flushDockerScan(httpMock, 'repo-1', 'my-app', 'latest', {
        scanned_at: '2026-01-05T00:00:00Z',
        vulnerabilities: [vuln('v-low', 'LOW')],
      })
      fixture.detectChanges()

      const filterTrigger: HTMLButtonElement =
        fixture.nativeElement.querySelector('[role="combobox"]')
      filterTrigger.click()
      fixture.detectChanges()
      const options: HTMLButtonElement[] = Array.from(
        fixture.nativeElement.querySelectorAll('[role="option"]'),
      )
      options.find((o) => o.textContent?.includes('Critique'))!.click()
      fixture.detectChanges()

      expect(fixture.nativeElement.textContent).toContain(
        'Aucune vulnérabilité ne correspond aux criticités sélectionnées',
      )
    })
  })

  describe('npm finding sorting and pagination', () => {
    function renderWithDetails() {
      const { fixture, httpMock } = render({ format: 'npm', name: 'left-pad' })
      httpMock.expectOne('/api/repositories/repo-1/packages/npm/left-pad').flush({
        name: 'left-pad',
        versions: [
          {
            version: '1.0.0',
            published_at: '2026-01-01T00:00:00Z',
            size_bytes: 1024,
            deprecated: false,
            deprecated_message: null,
            shasum: 'a',
          },
        ],
        dist_tags: [],
        readme_html: null,
        registry_url: 'http://localhost:4200/npm/u/alice/repo-1/',
      })
      flushAudit(httpMock, 'repo-1', 'left-pad', [])
      flushRepository(httpMock, 'repo-1')
      return { fixture, httpMock }
    }

    function finding(id: number, severity: string) {
      return {
        dependency_name: `dep-${id}`,
        dependency_version: '1.0.0',
        advisory: {
          id,
          url: 'https://example.com',
          title: `advisory-${id}`,
          severity,
          vulnerable_versions: '<1.0.0',
          cwe: [],
          cvss_score: null,
        },
      }
    }

    it('sorts findings from most to least critical', () => {
      const { fixture, httpMock } = renderWithDetails()
      flushDependencyAudit(httpMock, 'repo-1', 'left-pad', '1.0.0', {
        scanned_at: '2026-01-05T00:00:00Z',
        packages_scanned: 3,
        truncated: false,
        findings: [finding(1, 'low'), finding(2, 'critical'), finding(3, 'high')],
      })
      fixture.detectChanges()

      const severityElements: Element[] = Array.from(
        fixture.nativeElement.querySelectorAll('.package-detail__severity'),
      )
      const severities = severityElements.map((el) => el.textContent?.trim())
      expect(severities).toEqual(['critical', 'high', 'low'])
    })

    it('paginates the findings list at 20 per page', () => {
      const { fixture, httpMock } = renderWithDetails()
      const findings = Array.from({ length: 22 }, (_, i) => finding(i, 'low'))
      flushDependencyAudit(httpMock, 'repo-1', 'left-pad', '1.0.0', {
        scanned_at: '2026-01-05T00:00:00Z',
        packages_scanned: 22,
        truncated: false,
        findings,
      })
      fixture.detectChanges()

      expect(fixture.nativeElement.querySelectorAll('.package-detail__advisory').length).toBe(20)
      expect(fixture.nativeElement.textContent).toContain('Page 1 sur 2')
    })
  })

  it('hides delete and rescan actions for a read-only viewer of an npm package', () => {
    const { fixture, httpMock } = render({ format: 'npm', name: 'left-pad' })
    httpMock.expectOne('/api/repositories/repo-1/packages/npm/left-pad').flush({
      name: 'left-pad',
      versions: [
        {
          version: '1.0.0',
          published_at: '2026-01-01T00:00:00Z',
          size_bytes: 1024,
          deprecated: false,
          deprecated_message: null,
          shasum: 'a',
        },
      ],
      dist_tags: [],
      readme_html: null,
      registry_url: 'http://localhost:4200/npm/u/alice/repo-1/',
    })
    flushAudit(httpMock, 'repo-1', 'left-pad')
    flushDependencyAudit(httpMock, 'repo-1', 'left-pad', '1.0.0')
    flushRepository(httpMock, 'repo-1', 'read')
    fixture.detectChanges()

    expect(fixture.nativeElement.querySelector('[data-testid="delete-version-1.0.0"]')).toBeFalsy()
    const buttons: HTMLButtonElement[] = Array.from(
      fixture.nativeElement.querySelectorAll('gbt-button button'),
    )
    expect(buttons.some((b) => b.textContent?.includes('Supprimer tout le package'))).toBe(false)
    expect(buttons.some((b) => b.textContent?.includes('Relancer le scan'))).toBe(false)
    expect(buttons.some((b) => b.textContent?.includes('Lancer un scan approfondi'))).toBe(false)
  })

  it('hides delete and rescan actions for a read-only viewer of a docker image', () => {
    const { fixture, httpMock } = render({ format: 'docker', name: 'my-app' })
    httpMock.expectOne('/api/repositories/repo-1/packages/docker/my-app').flush({
      image_name: 'my-app',
      image_reference: 'localhost:4200/u/alice/repo-1/my-app',
      tags: [
        {
          tag: 'latest',
          digest: 'sha256:aaaa',
          media_type: 'application/vnd.docker.distribution.manifest.v2+json',
          created_at: '2026-01-01T00:00:00Z',
          size_bytes: null,
        },
      ],
    })
    flushDockerScan(httpMock, 'repo-1', 'my-app', 'latest')
    flushRepository(httpMock, 'repo-1', 'read')
    fixture.detectChanges()

    expect(fixture.nativeElement.querySelector('[data-testid="delete-tag-latest"]')).toBeFalsy()
    const buttons: HTMLButtonElement[] = Array.from(
      fixture.nativeElement.querySelectorAll('gbt-button button'),
    )
    expect(buttons.some((b) => b.textContent?.includes("Supprimer toute l'image"))).toBe(false)
    expect(buttons.some((b) => b.textContent?.includes('Lancer un scan'))).toBe(false)
  })

  it('does not apply a stale canWrite result after navigating to a different package before it resolves', () => {
    const { fixture, httpMock, paramMap$ } = renderNavigable({ id: 'repo-1', name: 'left-pad' })

    // Package A's requests are in flight — do not flush them yet.
    httpMock.expectOne('/api/repositories/repo-1/packages/npm/left-pad')
    const repoReqA = httpMock.expectOne('/api/repositories/repo-1')

    // Navigate to package B before A resolves.
    paramMap$.next(convertToParamMap({ id: 'repo-2', format: 'npm', name: 'right-pad' }))
    fixture.detectChanges()

    httpMock.expectOne('/api/repositories/repo-2/packages/npm/right-pad')
    const repoReqB = httpMock.expectOne('/api/repositories/repo-2')

    // Flush the CURRENT package's (B) response first, then the STALE package's (A)
    // response second — this is the actually racy order the stillCurrent() guard
    // exists to handle: a slow first request that finally resolves after a faster
    // second request has already applied its result.
    repoReqB.flush({
      id: 'repo-2',
      name: 'repo-2',
      format: 'npm',
      repo_type: 'hosted',
      remote_url: null,
      group_members: [],
      my_role: 'read',
    })
    repoReqA.flush({
      id: 'repo-1',
      name: 'repo-1',
      format: 'npm',
      repo_type: 'hosted',
      remote_url: null,
      group_members: [],
      my_role: 'write',
    })

    expect(fixture.componentInstance.canWrite()).toBe(false)
  })

  it('leaves canWrite at its safe default when the repository fetch errors', () => {
    const { fixture, httpMock } = renderNavigable({ id: 'repo-1', name: 'left-pad' })

    httpMock.expectOne('/api/repositories/repo-1/packages/npm/left-pad')
    httpMock
      .expectOne('/api/repositories/repo-1')
      .flush('network error', { status: 500, statusText: 'Internal Server Error' })

    expect(fixture.componentInstance.canWrite()).toBe(false)
  })

  describe('installation and README', () => {
    function renderNpm(readmeHtml: string | null) {
      const { fixture, httpMock } = render({ format: 'npm', name: 'button' })
      httpMock.expectOne('/api/repositories/repo-1/packages/npm/button').flush({
        name: 'button',
        versions: [],
        dist_tags: [{ tag: 'latest', version: '1.0.0' }],
        readme_html: readmeHtml,
        registry_url: 'http://localhost:4200/npm/u/alice/repo-1/',
      })
      flushAudit(httpMock, 'repo-1', 'button')
      fixture.detectChanges()
      return fixture.nativeElement as HTMLElement
    }

    function renderDocker(tags: { tag: string; size_bytes: number | null }[]) {
      const { fixture, httpMock } = render({ format: 'docker', name: 'hello' })
      httpMock.expectOne('/api/repositories/repo-1/packages/docker/hello').flush({
        image_name: 'hello',
        image_reference: 'localhost:4200/u/alice/repo-1/hello',
        tags: tags.map((t) => ({
          ...t,
          digest: 'sha256:aaaa',
          media_type: 'application/vnd.oci.image.manifest.v1+json',
          created_at: '2026-01-01T00:00:00Z',
        })),
      })
      flushDockerScan(
        httpMock,
        'repo-1',
        'hello',
        tags.some((t) => t.tag === 'latest') ? 'latest' : tags[0].tag,
      )
      fixture.detectChanges()
      return fixture.nativeElement as HTMLElement
    }

    it('shows the npm install command from the registry URL', () => {
      const el = renderNpm(null)

      expect(el.querySelector('app-copyable-command pre')!.textContent).toBe(
        'npm install button --registry http://localhost:4200/npm/u/alice/repo-1/',
      )
    })

    it('renders the README between the versions and the security card', () => {
      const el = renderNpm('<h2>Usage</h2><p>Call <code>button()</code></p>')

      const readme = el.querySelector('app-readme-view')!
      expect(readme.querySelector('.readme-view__content h2')!.textContent).toBe('Usage')
      const cards = Array.from(el.querySelectorAll(':scope .container > *'))
      const titles = cards.map((card) => card.textContent!.trim())
      const index = cards.indexOf(readme)
      expect(titles[index - 1]).toContain('version')
      expect(titles[index + 1]).toContain('Sécurité')
    })

    it('shows the empty README state when there is none', () => {
      const el = renderNpm(null)

      expect(el.querySelector('app-readme-view .readme-view__content')).toBeNull()
      expect(el.querySelector('app-readme-view')!.textContent).toContain('Aucun README')
    })

    it('shows the docker pull command with the latest tag and a size column', () => {
      const el = renderDocker([
        { tag: 'v1', size_bytes: 1_572_864 },
        { tag: 'latest', size_bytes: null },
      ])

      expect(el.querySelector('app-copyable-command pre')!.textContent).toBe(
        'docker pull localhost:4200/u/alice/repo-1/hello:latest',
      )
      expect(Array.from(el.querySelectorAll('th')).map((th) => th.textContent)).toContain('Taille')
      const sizes = Array.from(el.querySelectorAll('tbody tr')).map((row) =>
        row.querySelectorAll('td')[4].textContent!.trim(),
      )
      expect(sizes).toEqual(['1.5 Mo', '—'])
    })
  })

  describe('navigating from one package to another', () => {
    const NPM_DETAILS = (name: string) => ({
      name,
      versions: [
        {
          version: '1.0.0',
          published_at: '2026-01-01T00:00:00Z',
          size_bytes: 1024,
          deprecated: false,
          deprecated_message: null,
          shasum: 'a',
        },
      ],
      dist_tags: [{ tag: 'latest', version: '1.0.0' }],
      readme_html: null,
      registry_url: 'http://localhost:4200/npm/u/alice/repo-1/',
    })
    const ADVISORY = {
      id: 1,
      title: 'Prototype pollution in left-pad',
      severity: 'high',
      url: 'https://example.test/advisory/1',
      vulnerable_versions: '<1.0.0',
      cvss_score: null,
    }
    const FINDING = {
      package_name: 'evil-dep',
      package_version: '0.1.0',
      advisory: ADVISORY,
    }

    function navigateToB(
      fixture: { detectChanges(): void },
      paramMap$: BehaviorSubject<ReturnType<typeof convertToParamMap>>,
      params: { id: string; format: string; name: string },
    ) {
      paramMap$.next(convertToParamMap(params))
      fixture.detectChanges()
    }

    it("drops package A's late advisory response instead of showing it for package B", () => {
      const { fixture, httpMock, paramMap$ } = renderNavigable({ id: 'repo-1', name: 'left-pad' })
      httpMock.expectOne('/api/repositories/repo-1').flush({ my_role: 'read' })
      httpMock
        .expectOne('/api/repositories/repo-1/packages/npm/left-pad')
        .flush(NPM_DETAILS('left-pad'))
      const staleAudit = httpMock.expectOne('/api/repositories/repo-1/packages/npm/left-pad/audit')
      httpMock.expectOne(
        '/api/repositories/repo-1/packages/npm/left-pad/versions/1.0.0/dependency-audit',
      )

      navigateToB(fixture, paramMap$, { id: 'repo-1', format: 'npm', name: 'right-pad' })
      httpMock.expectOne('/api/repositories/repo-1')
      httpMock
        .expectOne('/api/repositories/repo-1/packages/npm/right-pad')
        .flush(NPM_DETAILS('right-pad'))
      const currentAudit = httpMock.expectOne(
        '/api/repositories/repo-1/packages/npm/right-pad/audit',
      )
      httpMock.expectOne(
        '/api/repositories/repo-1/packages/npm/right-pad/versions/1.0.0/dependency-audit',
      )

      currentAudit.flush([])
      staleAudit.flush([ADVISORY])
      fixture.detectChanges()

      expect(fixture.componentInstance.auditAdvisories()).toEqual([])
      expect(fixture.nativeElement.textContent).not.toContain('Prototype pollution')
    })

    it("keeps package B's advisory spinner running when only A's response arrives", () => {
      const { fixture, httpMock, paramMap$ } = renderNavigable({ id: 'repo-1', name: 'left-pad' })
      httpMock.expectOne('/api/repositories/repo-1').flush({ my_role: 'read' })
      httpMock
        .expectOne('/api/repositories/repo-1/packages/npm/left-pad')
        .flush(NPM_DETAILS('left-pad'))
      const staleAudit = httpMock.expectOne('/api/repositories/repo-1/packages/npm/left-pad/audit')
      httpMock.expectOne(
        '/api/repositories/repo-1/packages/npm/left-pad/versions/1.0.0/dependency-audit',
      )

      navigateToB(fixture, paramMap$, { id: 'repo-1', format: 'npm', name: 'right-pad' })
      httpMock.expectOne('/api/repositories/repo-1')
      httpMock
        .expectOne('/api/repositories/repo-1/packages/npm/right-pad')
        .flush(NPM_DETAILS('right-pad'))
      httpMock.expectOne('/api/repositories/repo-1/packages/npm/right-pad/audit')
      httpMock.expectOne(
        '/api/repositories/repo-1/packages/npm/right-pad/versions/1.0.0/dependency-audit',
      )

      staleAudit.flush([])
      fixture.detectChanges()

      expect(fixture.componentInstance.auditLoading()).toBe(true)
      expect(fixture.componentInstance.auditAdvisories()).toBeNull()
    })

    it("drops package A's late dependency-audit response", () => {
      const { fixture, httpMock, paramMap$ } = renderNavigable({ id: 'repo-1', name: 'left-pad' })
      httpMock.expectOne('/api/repositories/repo-1').flush({ my_role: 'read' })
      httpMock
        .expectOne('/api/repositories/repo-1/packages/npm/left-pad')
        .flush(NPM_DETAILS('left-pad'))
      httpMock.expectOne('/api/repositories/repo-1/packages/npm/left-pad/audit')
      const staleDepAudit = httpMock.expectOne(
        '/api/repositories/repo-1/packages/npm/left-pad/versions/1.0.0/dependency-audit',
      )

      navigateToB(fixture, paramMap$, { id: 'repo-1', format: 'npm', name: 'right-pad' })
      httpMock.expectOne('/api/repositories/repo-1')
      httpMock
        .expectOne('/api/repositories/repo-1/packages/npm/right-pad')
        .flush(NPM_DETAILS('right-pad'))
      httpMock.expectOne('/api/repositories/repo-1/packages/npm/right-pad/audit')
      const currentDepAudit = httpMock.expectOne(
        '/api/repositories/repo-1/packages/npm/right-pad/versions/1.0.0/dependency-audit',
      )

      currentDepAudit.flush(null)
      staleDepAudit.flush({
        findings: [FINDING],
        truncated: false,
        scanned_at: '2026-01-01T00:00:00Z',
      })
      fixture.detectChanges()

      expect(fixture.componentInstance.depAuditResult()).toBeNull()
    })

    it("clears A's dependency findings when B has no version to scan", () => {
      const { fixture, httpMock, paramMap$ } = renderNavigable({ id: 'repo-1', name: 'left-pad' })
      httpMock.expectOne('/api/repositories/repo-1').flush({ my_role: 'read' })
      httpMock
        .expectOne('/api/repositories/repo-1/packages/npm/left-pad')
        .flush(NPM_DETAILS('left-pad'))
      httpMock.expectOne('/api/repositories/repo-1/packages/npm/left-pad/audit').flush([ADVISORY])
      httpMock
        .expectOne('/api/repositories/repo-1/packages/npm/left-pad/versions/1.0.0/dependency-audit')
        .flush({ findings: [FINDING], truncated: false, scanned_at: '2026-01-01T00:00:00Z' })
      expect(fixture.componentInstance.depAuditResult()).not.toBeNull()

      navigateToB(fixture, paramMap$, { id: 'repo-1', format: 'npm', name: 'empty-pkg' })
      httpMock.expectOne('/api/repositories/repo-1')
      httpMock.expectOne('/api/repositories/repo-1/packages/npm/empty-pkg').flush({
        name: 'empty-pkg',
        versions: [],
        dist_tags: [],
        readme_html: null,
        registry_url: 'http://localhost:4200/npm/u/alice/repo-1/',
      })
      httpMock.expectOne('/api/repositories/repo-1/packages/npm/empty-pkg/audit').flush([])
      fixture.detectChanges()

      expect(fixture.componentInstance.depAuditResult()).toBeNull()
      expect(fixture.nativeElement.textContent).not.toContain('evil-dep')
    })

    it("drops the previous package's write access while the new repository loads", () => {
      const { fixture, httpMock, paramMap$ } = renderNavigable({ id: 'repo-1', name: 'left-pad' })
      httpMock.expectOne('/api/repositories/repo-1').flush({ my_role: 'admin' })
      httpMock.expectOne('/api/repositories/repo-1/packages/npm/left-pad')
      expect(fixture.componentInstance.canWrite()).toBe(true)

      navigateToB(fixture, paramMap$, { id: 'repo-2', format: 'npm', name: 'right-pad' })

      expect(fixture.componentInstance.canWrite()).toBe(false)
      httpMock.expectOne('/api/repositories/repo-2')
      httpMock.expectOne('/api/repositories/repo-2/packages/npm/right-pad')
    })

    it("drops image A's late scan response for image B", () => {
      const dockerDetails = (name: string) => ({
        image_name: name,
        image_reference: `localhost:4200/u/alice/repo-1/${name}`,
        tags: [
          {
            tag: 'latest',
            digest: 'sha256:aaaa',
            size_bytes: 1,
            media_type: 'application/vnd.oci.image.manifest.v1+json',
            created_at: '2026-01-01T00:00:00Z',
          },
        ],
      })
      const { fixture, httpMock, paramMap$ } = renderNavigable({
        id: 'repo-1',
        format: 'docker',
        name: 'alpha',
      })
      httpMock.expectOne('/api/repositories/repo-1').flush({ my_role: 'read' })
      httpMock
        .expectOne('/api/repositories/repo-1/packages/docker/alpha')
        .flush(dockerDetails('alpha'))
      const staleScan = httpMock.expectOne(
        '/api/repositories/repo-1/packages/docker/alpha/tags/latest/scan',
      )

      navigateToB(fixture, paramMap$, { id: 'repo-1', format: 'docker', name: 'beta' })
      httpMock.expectOne('/api/repositories/repo-1')
      httpMock
        .expectOne('/api/repositories/repo-1/packages/docker/beta')
        .flush(dockerDetails('beta'))
      const currentScan = httpMock.expectOne(
        '/api/repositories/repo-1/packages/docker/beta/tags/latest/scan',
      )

      currentScan.flush(null)
      staleScan.flush({
        scanned_at: '2026-01-01T00:00:00Z',
        vulnerabilities: [
          {
            id: 'CVE-2026-0001',
            package: 'openssl',
            installed_version: '1',
            fixed_version: null,
            severity: 'CRITICAL',
            title: 'bad',
            primary_url: null,
          },
        ],
      })
      fixture.detectChanges()

      expect(fixture.componentInstance.imageScanResult()).toBeNull()
    })
  })
})
