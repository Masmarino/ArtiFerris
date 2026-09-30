import { TestBed } from '@angular/core/testing'
import { provideRouter, Router, RoutesRecognized } from '@angular/router'
import { provideHttpClient } from '@angular/common/http'
import { provideHttpClientTesting } from '@angular/common/http/testing'
import { filter, firstValueFrom } from 'rxjs'
import { routes } from './app.routes'

// Identifies which loadComponent() a URL resolved to, without waiting for guards to pass or
// the component to actually load — RoutesRecognized fires once matching is done, before either.
// Each loadComponent closure's source text ends in `.then((m) => m.<ClassName>)`, which is enough
// to tell two routes apart without loading them. (The dynamic import's path argument itself gets
// compiled to a chunk reference under Vite, not the literal source string, so match on the class
// name rather than the file path.)
async function matchedComponentName(url: string): Promise<string> {
  TestBed.configureTestingModule({
    providers: [provideRouter(routes), provideHttpClient(), provideHttpClientTesting()],
  })
  const router = TestBed.inject(Router)
  const recognized = firstValueFrom(
    router.events.pipe(filter((e): e is RoutesRecognized => e instanceof RoutesRecognized)),
  )
  router.navigateByUrl(url).catch(() => {
    // A guard rejecting the navigation still fires RoutesRecognized first — that's all this test
    // needs.
  })
  const event = await recognized
  let route = event.state.root
  while (route.firstChild) {
    route = route.firstChild
  }
  return route.routeConfig?.loadComponent?.toString() ?? ''
}

describe('app.routes', () => {
  it('resolves a bare repository id to the authenticated RepositoryDetail, not the public page', async () => {
    const name = await matchedComponentName('/repositories/repo-1')

    expect(name).toContain('RepositoryDetail')
    expect(name).not.toContain('PublicRepositoryPage')
  })

  it('resolves an authenticated package-detail URL, not the public page', async () => {
    const name = await matchedComponentName('/repositories/repo-1/packages/npm/left-pad')

    expect(name).toContain('PackageDetailPage')
    expect(name).not.toContain('PublicPackagePage')
  })

  it('resolves a bare user id to the authenticated UserDetail, not the public page', async () => {
    const name = await matchedComponentName('/users/user-1')

    expect(name).toContain('UserDetail')
  })

  it('resolves an admin sub-route, not the public page', async () => {
    const name = await matchedComponentName('/admin/export')

    expect(name).toContain('ExportAdmin')
  })

  it('still resolves a genuine @username/repoName URL to the public repository page', async () => {
    const name = await matchedComponentName('/@alice/my-lib')

    expect(name).toContain('PublicRepositoryPage')
  })

  it('still resolves a genuine public package-detail URL to the public package page', async () => {
    const name = await matchedComponentName('/@alice/my-lib/packages/npm/left-pad')

    expect(name).toContain('PublicPackagePage')
  })

  it('resolves /explorer to the explorer page', async () => {
    expect(await matchedComponentName('/explorer')).toContain('ExplorerPage')
  })

  it('resolves the home page (/) to the explorer page, not the authenticated shell', async () => {
    const name = await matchedComponentName('/')

    expect(name).toContain('ExplorerPage')
    expect(name).not.toContain('AppShell')
  })

  it('resolves each catalog URL to the catalog page', async () => {
    expect(await matchedComponentName('/artiferris-npm')).toContain('CatalogPage')
    TestBed.resetTestingModule()
    expect(await matchedComponentName('/artiferris-docker?q=web')).toContain('CatalogPage')
  })

  it('resolves an organization repository URL to the public repository page', async () => {
    const name = await matchedComponentName('/o/acme/my-lib')

    expect(name).toContain('PublicRepositoryPage')
  })

  it('resolves an organization package URL to the public package page', async () => {
    const name = await matchedComponentName('/o/acme/images/packages/docker/team%2Fapi')

    expect(name).toContain('PublicPackagePage')
  })

  it('does not let the o/ routes shadow authenticated ones', async () => {
    expect(await matchedComponentName('/admin/organizations/org-1')).toContain('OrganizationsPage')
  })

  it('resolves @username to the owner page', async () => {
    expect(await matchedComponentName('/@alice')).toContain('OwnerPage')
  })

  it('resolves /o/:slug to the owner page, not the public repository page', async () => {
    const name = await matchedComponentName('/o/acme')

    expect(name).toContain('OwnerPage')
    expect(name).not.toContain('PublicRepositoryPage')
  })

  it('hands the username without the @ to the owner page', async () => {
    TestBed.configureTestingModule({
      providers: [provideRouter(routes), provideHttpClient(), provideHttpClientTesting()],
    })
    const router = TestBed.inject(Router)
    const recognized = firstValueFrom(
      router.events.pipe(filter((e): e is RoutesRecognized => e instanceof RoutesRecognized)),
    )
    void router.navigateByUrl('/@alice')

    const event = await recognized

    expect(event.state.root.firstChild?.paramMap.get('username')).toBe('alice')
  })

  it('still resolves @username/repoName to the repository page, not the owner page', async () => {
    expect(await matchedComponentName('/@alice/my-lib')).toContain('PublicRepositoryPage')
  })

  it('keeps /o/:slug/:repoName on the repository page', async () => {
    expect(await matchedComponentName('/o/acme/images')).toContain('PublicRepositoryPage')
  })

  it('does not let the owner routes shadow the authenticated ones', async () => {
    expect(await matchedComponentName('/users/user-1')).toContain('UserDetail')
    TestBed.resetTestingModule()
    expect(await matchedComponentName('/account')).toContain('AccountPage')
  })

  it('sends unknown URLs of any depth to the not-found page', async () => {
    expect(await matchedComponentName('/foo')).toContain('NotFoundPage')
    TestBed.resetTestingModule()
    expect(await matchedComponentName('/o/a/b/c')).toContain('NotFoundPage')
    TestBed.resetTestingModule()
    expect(await matchedComponentName('/@a/b/packages/npm')).toContain('NotFoundPage')
    TestBed.resetTestingModule()
    expect(await matchedComponentName('/a/b/c/d/e/f')).toContain('NotFoundPage')
  })

  it('does not let the not-found route shadow real ones', async () => {
    for (const [url, component] of [
      ['/repositories', 'RepositoriesList'],
      ['/repositories/repo-1', 'RepositoryDetail'],
      ['/admin/organizations/org-1', 'OrganizationsPage'],
      ['/@alice', 'OwnerPage'],
      ['/o/acme/images', 'PublicRepositoryPage'],
      ['/explorer', 'ExplorerPage'],
      ['/login', 'LoginPage'],
    ]) {
      TestBed.resetTestingModule()
      expect(await matchedComponentName(url)).toContain(component)
    }
  })

  it.each([
    '/repositories/repo-1/packages/pypi/left-pad',
    '/@alice/lib/packages/pypi/left-pad',
    '/o/acme/lib/packages/pypi/left-pad',
  ])('sends %s, a format nobody serves, to the not-found page', async (url) => {
    expect(await matchedComponentName(url)).toContain('NotFoundPage')
  })

  it.each([
    ['/@alice/lib/packages/npm/left-pad', 'PublicPackagePage'],
    ['/o/acme/lib/packages/docker/api', 'PublicPackagePage'],
  ])('still serves %s', async (url, page) => {
    expect(await matchedComponentName(url)).toContain(page)
  })
})
