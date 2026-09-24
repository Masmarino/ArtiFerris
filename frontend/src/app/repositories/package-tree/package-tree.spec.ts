import { TestBed } from '@angular/core/testing'
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing'
import { provideHttpClient } from '@angular/common/http'
import { provideRouter } from '@angular/router'
import { PackageTree } from './package-tree'
import { repositoryProviders } from '../infrastructure/repository.providers'

function render(repositoryId: string, basePath?: string[]) {
  TestBed.configureTestingModule({
    providers: [
      provideHttpClient(),
      provideHttpClientTesting(),
      provideRouter([]),
      ...repositoryProviders,
    ],
  })
  const fixture = TestBed.createComponent(PackageTree)
  fixture.componentRef.setInput('repositoryId', repositoryId)
  if (basePath !== undefined) {
    fixture.componentRef.setInput('basePath', basePath)
  }
  fixture.detectChanges()
  const httpMock = TestBed.inject(HttpTestingController)
  return { fixture, httpMock }
}

describe('PackageTree', () => {
  it('fetches the tree for the given repository id', () => {
    const { httpMock } = render('repo-1')

    httpMock.expectOne('/api/repositories/repo-1/packages').flush({ format: 'npm', packages: [] })
  })

  it('shows an empty state when an npm repository has no packages', () => {
    const { fixture, httpMock } = render('repo-1')
    httpMock.expectOne('/api/repositories/repo-1/packages').flush({ format: 'npm', packages: [] })
    fixture.detectChanges()

    expect(fixture.nativeElement.querySelector('.gbt-empty-state__heading').textContent).toBe(
      'Aucun package',
    )
  })

  it('shows an empty state when a docker repository has no images', () => {
    const { fixture, httpMock } = render('repo-1')
    httpMock.expectOne('/api/repositories/repo-1/packages').flush({ format: 'docker', images: [] })
    fixture.detectChanges()

    expect(fixture.nativeElement.querySelector('.gbt-empty-state__heading').textContent).toBe(
      'Aucune image',
    )
  })

  it('lists npm packages with their version count', () => {
    const { fixture, httpMock } = render('repo-1')
    httpMock.expectOne('/api/repositories/repo-1/packages').flush({
      format: 'npm',
      packages: [
        {
          name: 'left-pad',
          versions: [
            {
              version: '1.0.0',
              published_at: '2026-01-01T00:00:00Z',
              size_bytes: 10,
              deprecated: false,
            },
          ],
          vulnerability_summary: { critical: 0, high: 0, medium: 0, low: 0 },
        },
      ],
    })
    fixture.detectChanges()

    const text = fixture.nativeElement.textContent as string
    expect(text).toContain('left-pad')
    expect(text).toContain('1')
  })

  it('links a plain npm package name to its detail page', () => {
    const { fixture, httpMock } = render('repo-1')
    httpMock.expectOne('/api/repositories/repo-1/packages').flush({
      format: 'npm',
      packages: [
        {
          name: 'left-pad',
          versions: [
            {
              version: '1.0.0',
              published_at: '2026-01-01T00:00:00Z',
              size_bytes: 10,
              deprecated: false,
            },
          ],
          vulnerability_summary: { critical: 0, high: 0, medium: 0, low: 0 },
        },
      ],
    })
    fixture.detectChanges()

    const link: HTMLAnchorElement = fixture.nativeElement.querySelector('.package-tree__node')
    expect(link.getAttribute('href')).toBe('/repositories/repo-1/packages/npm/left-pad')
  })

  it('percent-encodes a scoped npm package name so it stays a single route segment', () => {
    const { fixture, httpMock } = render('repo-1')
    httpMock.expectOne('/api/repositories/repo-1/packages').flush({
      format: 'npm',
      packages: [
        {
          name: '@scope/name',
          versions: [
            {
              version: '1.0.0',
              published_at: '2026-01-01T00:00:00Z',
              size_bytes: 10,
              deprecated: false,
            },
          ],
          vulnerability_summary: { critical: 0, high: 0, medium: 0, low: 0 },
        },
      ],
    })
    fixture.detectChanges()

    const link: HTMLAnchorElement = fixture.nativeElement.querySelector('.package-tree__node')
    expect(link.getAttribute('href')).toBe('/repositories/repo-1/packages/npm/@scope%2Fname')
  })

  it('links under a custom basePath when provided', () => {
    const { fixture, httpMock } = render('repo-1', ['/@alice', 'my-lib'])
    httpMock.expectOne('/api/repositories/repo-1/packages').flush({
      format: 'npm',
      packages: [
        {
          name: 'left-pad',
          versions: [
            {
              version: '1.0.0',
              published_at: '2026-01-01T00:00:00Z',
              size_bytes: 10,
              deprecated: false,
            },
          ],
          vulnerability_summary: { critical: 0, high: 0, medium: 0, low: 0 },
        },
      ],
    })
    fixture.detectChanges()

    const link: HTMLAnchorElement = fixture.nativeElement.querySelector('.package-tree__node')
    expect(link.getAttribute('href')).toBe('/@alice/my-lib/packages/npm/left-pad')
  })

  it('links a docker image under a custom basePath when provided', () => {
    const { fixture, httpMock } = render('repo-1', ['/@alice', 'my-lib'])
    httpMock.expectOne('/api/repositories/repo-1/packages').flush({
      format: 'docker',
      images: [
        {
          image_name: 'my-app',
          tags: ['latest'],
          vulnerability_summary: { critical: 0, high: 0, medium: 0, low: 0 },
        },
      ],
    })
    fixture.detectChanges()

    const link: HTMLAnchorElement = fixture.nativeElement.querySelector('.package-tree__node')
    expect(link.getAttribute('href')).toBe('/@alice/my-lib/packages/docker/my-app')
  })

  it('links a docker image to its detail page', () => {
    const { fixture, httpMock } = render('repo-1')
    httpMock.expectOne('/api/repositories/repo-1/packages').flush({
      format: 'docker',
      images: [
        {
          image_name: 'my-app',
          tags: ['latest'],
          vulnerability_summary: { critical: 0, high: 0, medium: 0, low: 0 },
        },
      ],
    })
    fixture.detectChanges()

    const link: HTMLAnchorElement = fixture.nativeElement.querySelector('.package-tree__node')
    expect(link.getAttribute('href')).toBe('/repositories/repo-1/packages/docker/my-app')
  })

  it('shows one vulnerability circle per severity on a docker row, counts included', () => {
    const { fixture, httpMock } = render('repo-1')
    httpMock.expectOne('/api/repositories/repo-1/packages').flush({
      format: 'docker',
      images: [
        {
          image_name: 'my-app',
          tags: ['latest'],
          vulnerability_summary: { critical: 1, high: 2, medium: 0, low: 0 },
        },
      ],
    })
    fixture.detectChanges()

    const circles: NodeListOf<HTMLSpanElement> =
      fixture.nativeElement.querySelectorAll('.vuln-summary__circle')
    expect(circles.length).toBe(4)
    expect(circles[0].textContent?.trim()).toBe('1')
    expect(circles[0].classList).toContain('vuln-summary__circle--critical')
    expect(circles[1].textContent?.trim()).toBe('2')
    expect(circles[1].classList).toContain('vuln-summary__circle--high')
    expect(circles[2].textContent?.trim()).toBe('0')
    expect(circles[3].textContent?.trim()).toBe('0')
  })

  it('still shows all four vulnerability circles, at zero, when a package has a clean summary', () => {
    const { fixture, httpMock } = render('repo-1')
    httpMock.expectOne('/api/repositories/repo-1/packages').flush({
      format: 'npm',
      packages: [
        {
          name: 'left-pad',
          versions: [
            {
              version: '1.0.0',
              published_at: '2026-01-01T00:00:00Z',
              size_bytes: 10,
              deprecated: false,
            },
          ],
          vulnerability_summary: { critical: 0, high: 0, medium: 0, low: 0 },
        },
      ],
    })
    fixture.detectChanges()

    const circles: NodeListOf<HTMLSpanElement> =
      fixture.nativeElement.querySelectorAll('.vuln-summary__circle')
    expect(circles.length).toBe(4)
    expect(Array.from(circles).every((c) => c.textContent?.trim() === '0')).toBe(true)
  })

  describe('truncated entries', () => {
    const summary = { critical: 0, high: 0, medium: 0, low: 0 }
    const hints = (fixture: { nativeElement: HTMLElement }) =>
      Array.from(fixture.nativeElement.querySelectorAll('[data-testid="truncated-hint"]')).map(
        (el) => el.textContent?.trim(),
      )

    it('tells which npm package lists only its most recent versions', () => {
      const { fixture, httpMock } = render('repo-1')
      const versions = (count: number) =>
        Array.from({ length: count }, (_, i) => ({
          version: `1.0.${i}`,
          published_at: '2026-01-01T00:00:00Z',
          size_bytes: 10,
          deprecated: false,
        }))
      httpMock.expectOne('/api/repositories/repo-1/packages').flush({
        format: 'npm',
        packages: [
          {
            name: 'busy',
            versions: versions(200),
            truncated: true,
            vulnerability_summary: summary,
          },
          { name: 'calm', versions: versions(2), truncated: false, vulnerability_summary: summary },
        ],
      })
      fixture.detectChanges()

      expect(hints(fixture)).toEqual(['Seules les 200 plus récentes sont listées'])
    })

    it('tells which docker image lists only its most recent tags', () => {
      const { fixture, httpMock } = render('repo-1')
      httpMock.expectOne('/api/repositories/repo-1/packages').flush({
        format: 'docker',
        images: [
          {
            image_name: 'busy',
            tags: Array.from({ length: 100 }, (_, i) => `t${i}`),
            truncated: true,
            vulnerability_summary: summary,
          },
          {
            image_name: 'calm',
            tags: ['latest'],
            truncated: false,
            vulnerability_summary: summary,
          },
        ],
      })
      fixture.detectChanges()

      expect(hints(fixture)).toEqual(['Seules les 100 plus récentes sont listées'])
    })
  })

  it('re-fetches when the repository id input changes', () => {
    const { fixture, httpMock } = render('repo-1')
    httpMock.expectOne('/api/repositories/repo-1/packages').flush({ format: 'npm', packages: [] })

    fixture.componentRef.setInput('repositoryId', 'repo-2')
    fixture.detectChanges()

    httpMock.expectOne('/api/repositories/repo-2/packages').flush({ format: 'npm', packages: [] })
  })

  describe('paging', () => {
    const npmPackage = (name: string) => ({
      name,
      versions: [],
      vulnerability_summary: { critical: 0, high: 0, medium: 0, low: 0 },
    })
    const rows = (fixture: { nativeElement: HTMLElement }) =>
      Array.from(fixture.nativeElement.querySelectorAll('.package-tree__label')).map((e) =>
        e.textContent!.trim(),
      )
    const moreButton = (fixture: { nativeElement: HTMLElement }) =>
      Array.from(fixture.nativeElement.querySelectorAll('button')).find((b) =>
        b.textContent!.includes('Charger plus'),
      ) as HTMLButtonElement | undefined

    it('offers no "Charger plus" on the last page', () => {
      const { fixture, httpMock } = render('repo-1')
      httpMock
        .expectOne('/api/repositories/repo-1/packages')
        .flush({ format: 'npm', packages: [npmPackage('a')], next_after: null })
      fixture.detectChanges()

      expect(moreButton(fixture)).toBeUndefined()
    })

    it('appends the next npm page after the cursor of the previous one', () => {
      const { fixture, httpMock } = render('repo-1')
      httpMock
        .expectOne('/api/repositories/repo-1/packages')
        .flush({ format: 'npm', packages: [npmPackage('a'), npmPackage('b')], next_after: 'b' })
      fixture.detectChanges()

      moreButton(fixture)!.click()
      const req = httpMock.expectOne((r) => r.url === '/api/repositories/repo-1/packages')
      expect(req.request.params.get('after')).toBe('b')
      req.flush({ format: 'npm', packages: [npmPackage('c')], next_after: null })
      fixture.detectChanges()

      expect(rows(fixture)).toEqual(['a', 'b', 'c'])
      expect(moreButton(fixture)).toBeUndefined()
    })

    it('appends the next docker page and keeps offering more while a cursor is returned', () => {
      const { fixture, httpMock } = render('repo-1')
      const image = (name: string) => ({
        image_name: name,
        tags: ['latest'],
        vulnerability_summary: { critical: 0, high: 0, medium: 0, low: 0 },
      })
      httpMock
        .expectOne('/api/repositories/repo-1/packages')
        .flush({ format: 'docker', images: [image('a')], next_after: 'a' })
      fixture.detectChanges()

      moreButton(fixture)!.click()
      httpMock
        .expectOne((r) => r.url === '/api/repositories/repo-1/packages')
        .flush({ format: 'docker', images: [image('b')], next_after: 'b' })
      fixture.detectChanges()

      expect(rows(fixture)).toEqual(['a', 'b'])
      expect(moreButton(fixture)).toBeDefined()
    })

    it('keeps the loaded rows and shows an error when a further page fails, then retries', () => {
      const { fixture, httpMock } = render('repo-1')
      httpMock
        .expectOne('/api/repositories/repo-1/packages')
        .flush({ format: 'npm', packages: [npmPackage('a')], next_after: 'a' })
      fixture.detectChanges()

      moreButton(fixture)!.click()
      httpMock
        .expectOne((r) => r.url === '/api/repositories/repo-1/packages')
        .flush(null, { status: 500, statusText: 'Server Error' })
      fixture.detectChanges()

      expect(rows(fixture)).toEqual(['a'])
      expect(fixture.nativeElement.querySelector('[role="alert"]').textContent).toContain(
        'Échec du chargement de la suite',
      )

      moreButton(fixture)!.click()
      httpMock
        .expectOne((r) => r.url === '/api/repositories/repo-1/packages')
        .flush({ format: 'npm', packages: [npmPackage('b')], next_after: null })
      fixture.detectChanges()

      expect(rows(fixture)).toEqual(['a', 'b'])
      expect(fixture.nativeElement.querySelector('[role="alert"]')).toBeNull()
    })

    it('ignores a late page for the previous repository after the id changed', () => {
      const { fixture, httpMock } = render('repo-1')
      httpMock
        .expectOne('/api/repositories/repo-1/packages')
        .flush({ format: 'npm', packages: [npmPackage('a')], next_after: 'a' })
      fixture.detectChanges()
      moreButton(fixture)!.click()
      const late = httpMock.expectOne((r) => r.url === '/api/repositories/repo-1/packages')

      fixture.componentRef.setInput('repositoryId', 'repo-2')
      fixture.detectChanges()
      httpMock
        .expectOne('/api/repositories/repo-2/packages')
        .flush({ format: 'npm', packages: [npmPackage('x')], next_after: null })
      late.flush({ format: 'npm', packages: [npmPackage('late')], next_after: null })
      fixture.detectChanges()

      expect(rows(fixture)).toEqual(['x'])
    })
  })
})
