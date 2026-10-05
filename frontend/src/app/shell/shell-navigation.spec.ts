import { TestBed } from '@angular/core/testing'
import { provideHttpClient, withInterceptors } from '@angular/common/http'
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing'
import { Router, provideRouter } from '@angular/router'
import { RouterTestingHarness } from '@angular/router/testing'
import { routes } from '../app.routes'
import { authInterceptor } from '../auth/auth.interceptor'
import { AuthService } from '../auth/application/auth.service'
import { apiTokenProviders } from '../tokens/infrastructure/api-token.providers'
import { authProviders } from '../auth/infrastructure/auth.providers'
import { userProviders } from '../users/infrastructure/user.providers'
import { repositoryProviders } from '../repositories/infrastructure/repository.providers'
import { adminProviders } from '../admin/infrastructure/admin.providers'
import { meProviders } from './infrastructure/me.providers'
import { versionProviders } from './infrastructure/version.providers'
import { readableCatalogProviders } from './infrastructure/readable-catalog.providers'
import { organizationsProviders } from '../admin/infrastructure/organizations.providers'
import { organizationMembersProviders } from '../admin/infrastructure/organization-members.providers'
import { catalogProviders } from '../public/catalog/infrastructure/catalog.providers'

/**
 * The app's own routes, as a signed-in user meets them: the shell is created while the navigation to
 * a page under it is under way (its specs create it first, then navigate). A throw there cancels the
 * navigation and the app stays where it was.
 */
// Each page is a lazy chunk: loading the first ones takes a few seconds when the whole suite runs at once.
describe('navigating under the shell, with the app routes', { timeout: 30_000 }, () => {
  async function signedIn(url: string) {
    TestBed.configureTestingModule({
      providers: [
        provideRouter(routes),
        provideHttpClient(withInterceptors([authInterceptor])),
        provideHttpClientTesting(),
        ...apiTokenProviders,
        ...authProviders,
        ...userProviders,
        ...organizationsProviders,
        ...organizationMembersProviders,
        ...repositoryProviders,
        ...catalogProviders,
        ...adminProviders,
        ...meProviders,
        ...versionProviders,
        ...readableCatalogProviders,
      ],
    })
    TestBed.inject(AuthService).token.set('a-token')
    const http = TestBed.inject(HttpTestingController)
    const harness = await RouterTestingHarness.create()
    const root = harness.fixture.nativeElement as HTMLElement

    /** Answers whatever the guards and pages ask (a super-admin's session, empty lists) until they are done. */
    const answer = () => {
      for (const req of http.match(() => true).filter((pending) => !pending.cancelled)) {
        if (req.request.url === '/api/me') {
          req.flush({
            id: 'u1',
            username: 'florian',
            is_super_admin: true,
            is_organization_admin: false,
            organization_id: null,
            created_at: '2026-01-01T00:00:00Z',
            language: 'fr',
          })
        } else if (req.request.url === '/api/version') {
          req.flush({ version: '0.1.0' })
        } else {
          req.flush([])
        }
      }
      harness.detectChanges()
    }
    const settle = async () => {
      for (let round = 0; round < 10; round++) {
        await new Promise((resolve) => setTimeout(resolve, 20))
        answer()
      }
    }

    await harness.navigateByUrl(url)
    await settle()
    const h1s = () => Array.from(root.querySelectorAll('h1')).map((h) => h.textContent?.trim())
    const router = TestBed.inject(Router)
    /** Clicks a rail link, then answers its guard and its page until the navigation is over. */
    const follow = async (href: string) => {
      root.querySelector<HTMLAnchorElement>(`a.gbt-app-shell__link[href="${href}"]`)!.click()
      for (let waited = 0; router.url !== href && waited < 10_000; waited += 20) {
        await new Promise((resolve) => setTimeout(resolve, 20))
        answer()
      }
      await settle()
    }
    return { router, h1s, follow }
  }

  it('opens a page under the shell on its own heading, the one h1', async () => {
    const { router, h1s } = await signedIn('/repositories')

    expect(router.url).toBe('/repositories')
    expect(h1s()).toEqual(['Dépôts'])
  })

  it('goes from page to page through the rail', async () => {
    const { router, h1s, follow } = await signedIn('/repositories')

    await follow('/users')
    expect(router.url).toBe('/users')
    expect(h1s()).toEqual(['Utilisateurs'])

    await follow('/my-repository')
    expect(router.url).toBe('/my-repository')
    expect(h1s()).toEqual(['Mon dépôt'])
  })
})
