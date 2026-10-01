import { vi } from 'vitest'
import { Component } from '@angular/core'
import { TestBed } from '@angular/core/testing'
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing'
import { provideHttpClient } from '@angular/common/http'
import { Router, provideRouter } from '@angular/router'
import { LanguageService } from '../shared/i18n/language.service'
import { registerTranslator } from '../shared/i18n/translator'
import { AppShell } from './app-shell'
import { AuthService } from '../auth/application/auth.service'
import { meProviders } from './infrastructure/me.providers'
import { versionProviders } from './infrastructure/version.providers'
import { readableCatalogProviders } from './infrastructure/readable-catalog.providers'
import {
  proxiedEntry,
  readableDockerEntry,
  readableEntry,
  readableSearchResult,
} from './testing/readable-catalog.fixtures'
import { authProviders } from '../auth/infrastructure/auth.providers'
import { userProviders } from '../users/infrastructure/user.providers'
import { repositoryProviders } from '../repositories/infrastructure/repository.providers'

@Component({ standalone: true, template: '' })
class DummyRoutedComponent {}

describe('AppShell', () => {
  let httpMock: HttpTestingController

  beforeEach(() => {
    TestBed.configureTestingModule({
      imports: [AppShell],
      providers: [
        provideHttpClient(),
        provideHttpClientTesting(),
        ...authProviders,
        ...userProviders,
        ...repositoryProviders,
        ...meProviders,
        ...versionProviders,
        ...readableCatalogProviders,
        provideRouter([]),
      ],
    })
    httpMock = TestBed.inject(HttpTestingController)
  })

  afterEach(() => {
    httpMock.verify()
  })

  function flushMe(me: {
    id: string
    username: string
    is_super_admin: boolean
    is_organization_admin?: boolean
    organization_id?: string
  }) {
    httpMock.expectOne('/api/version').flush({ version: '0.2.3' })
    httpMock
      .expectOne('/api/me')
      .flush({ is_organization_admin: false, organization_id: 'org-1', ...me })
    httpMock.expectOne('/api/repositories').flush([])
    // Super-admins and organization admins both see the Utilisateurs category, so
    // refreshSearchData() fires /api/users.
    if (me.is_super_admin || me.is_organization_admin) {
      httpMock.expectOne('/api/users').flush([])
    }
  }

  it('renders a skip-link (from gbt-app-shell) targeting the actual main content element', () => {
    const fixture = TestBed.createComponent(AppShell)
    fixture.detectChanges()
    flushMe({ id: 'user-1', username: 'florian', is_super_admin: false })
    fixture.detectChanges()

    const skipLink: HTMLAnchorElement = fixture.nativeElement.querySelector('.gbt-app-shell__skip')
    expect(skipLink.textContent?.trim()).toBe('Aller au contenu principal')

    const targetId = skipLink.getAttribute('href')!.replace('#', '')
    const main = fixture.nativeElement.querySelector(`#${targetId}`)
    expect(main).toBeTruthy()
    expect(main.classList.contains('gbt-app-shell__content')).toBe(true)
  })

  it('renders the mobile nav toggle (from gbt-app-shell), hidden by default via CSS but present in the DOM', () => {
    const fixture = TestBed.createComponent(AppShell)
    fixture.detectChanges()
    flushMe({ id: 'user-1', username: 'florian', is_super_admin: false })
    fixture.detectChanges()

    const toggle: HTMLButtonElement = fixture.nativeElement.querySelector('.gbt-app-shell__toggle')
    expect(toggle).toBeTruthy()
    expect(toggle.getAttribute('aria-label')).toBe('Ouvrir le menu de navigation')
    expect(toggle.getAttribute('aria-expanded')).toBe('false')
  })

  it('shows the server version in the sidebar once /api/version resolves', () => {
    const fixture = TestBed.createComponent(AppShell)
    fixture.detectChanges()

    expect(fixture.nativeElement.textContent).not.toContain('v0.2.3')

    flushMe({ id: 'user-1', username: 'florian', is_super_admin: false })
    fixture.detectChanges()

    expect(fixture.nativeElement.textContent).toContain('v0.2.3')
  })

  it('excludes admin-only items from navItems before /api/me resolves', () => {
    const fixture = TestBed.createComponent(AppShell)
    fixture.detectChanges()

    const actions = fixture.componentInstance.navItems().map((item) => item.action)
    expect(actions).toEqual(['explorer', 'repositories', 'my-repository'])

    flushMe({ id: 'user-1', username: 'florian', is_super_admin: false })
  })

  it('includes admin-only items in navItems once /api/me resolves with is_super_admin: true', () => {
    const fixture = TestBed.createComponent(AppShell)
    fixture.detectChanges()

    flushMe({ id: 'user-1', username: 'florian', is_super_admin: true })
    fixture.detectChanges()

    const actions = fixture.componentInstance.navItems().map((item) => item.action)
    expect(actions).toEqual(['explorer', 'repositories', 'my-repository', 'users', 'admin'])

    const adminItem = fixture.componentInstance.navItems().find((item) => item.action === 'admin')!
    expect(adminItem.children?.map((child) => child.action)).toEqual([
      'organizations',
      'export',
      'health',
    ])
  })

  it('keeps admin-only items hidden when /api/me resolves with is_super_admin: false', () => {
    const fixture = TestBed.createComponent(AppShell)
    fixture.detectChanges()

    flushMe({ id: 'user-1', username: 'florian', is_super_admin: false })
    fixture.detectChanges()

    const actions = fixture.componentInstance.navItems().map((item) => item.action)
    expect(actions).toEqual(['explorer', 'repositories', 'my-repository'])
  })

  it('always includes the "Mon dépôt" item, regardless of super-admin or staff status', () => {
    const fixture = TestBed.createComponent(AppShell)
    fixture.detectChanges()
    flushMe({ id: 'user-1', username: 'florian', is_super_admin: false })
    fixture.detectChanges()

    const actions = fixture.componentInstance.navItems().map((item) => item.action)
    expect(actions).toContain('my-repository')
  })

  it('includes the "Mon dépôt" item for a super-admin too', () => {
    const fixture = TestBed.createComponent(AppShell)
    fixture.detectChanges()
    flushMe({ id: 'user-1', username: 'admin', is_super_admin: true })
    fixture.detectChanges()

    const actions = fixture.componentInstance.navItems().map((item) => item.action)
    expect(actions).toContain('my-repository')
  })

  // "Explorer" links to '/', which non-exact routerLinkActive would light up everywhere: it needs
  // an exact match.
  it('never marks the "Explorer" item active while on an authenticated page', async () => {
    TestBed.resetTestingModule()
    TestBed.configureTestingModule({
      imports: [AppShell],
      providers: [
        provideHttpClient(),
        provideHttpClientTesting(),
        ...authProviders,
        ...userProviders,
        ...repositoryProviders,
        ...meProviders,
        ...versionProviders,
        ...readableCatalogProviders,
        provideRouter([{ path: 'repositories', component: DummyRoutedComponent }]),
      ],
    })
    httpMock = TestBed.inject(HttpTestingController)
    const fixture = TestBed.createComponent(AppShell)
    fixture.detectChanges()
    flushMe({ id: 'user-1', username: 'florian', is_super_admin: false })
    fixture.detectChanges()

    const router = TestBed.inject(Router)
    await router.navigateByUrl('/repositories')
    fixture.detectChanges()
    await fixture.whenStable()
    fixture.detectChanges()

    const links: HTMLAnchorElement[] = Array.from(
      fixture.nativeElement.querySelectorAll('a.app-shell__nav-link'),
    )
    const explorerLink = links.find((el) => el.textContent?.includes('Explorer'))!
    const repositoriesLink = links.find((el) => el.textContent?.includes('Dépôts'))!

    expect(explorerLink.classList.contains('app-shell__nav-link--active')).toBe(false)
    expect(repositoriesLink.classList.contains('app-shell__nav-link--active')).toBe(true)
  })

  it('shows the Utilisateurs item for an organization admin, even though they are not a super-admin', () => {
    const fixture = TestBed.createComponent(AppShell)
    fixture.detectChanges()

    flushMe({
      id: 'user-1',
      username: 'org-admin',
      is_super_admin: false,
      is_organization_admin: true,
      organization_id: 'org-1',
    })
    fixture.detectChanges()

    const actions = fixture.componentInstance.navItems().map((item) => item.action)
    expect(actions).toContain('users')
  })

  it('shows an Administration link for an org-admin who is not a super-admin, linking to their own organization', () => {
    const fixture = TestBed.createComponent(AppShell)
    fixture.detectChanges()
    flushMe({
      id: 'user-1',
      username: 'org-admin',
      is_super_admin: false,
      is_organization_admin: true,
      organization_id: 'org-1',
    })
    fixture.detectChanges()

    const actions = fixture.componentInstance.navItems().map((item) => item.action)
    expect(actions).toEqual(['explorer', 'repositories', 'my-repository', 'users', 'organization'])
    const orgItem = fixture.componentInstance
      .navItems()
      .find((item) => item.action === 'organization')!
    expect(orgItem.text).toBe('Administration')
    expect(orgItem.link).toBe('/admin/organizations/org-1')
    expect(orgItem.children).toBeUndefined()
  })

  it('does not show the Administration submenu for a super-admin, who already reaches every organization', () => {
    const fixture = TestBed.createComponent(AppShell)
    fixture.detectChanges()
    flushMe({
      id: 'user-1',
      username: 'admin',
      is_super_admin: true,
      is_organization_admin: true,
      organization_id: 'org-1',
    })
    fixture.detectChanges()

    const actions = fixture.componentInstance.navItems().map((item) => item.action)
    expect(actions).not.toContain('organization')
  })

  it('a super-admin never gets a standing "organization" nav item — every organization is reached via the Organisations list instead', () => {
    const fixture = TestBed.createComponent(AppShell)
    fixture.detectChanges()
    flushMe({ id: 'user-1', username: 'admin', is_super_admin: true })
    fixture.detectChanges()

    const actions = fixture.componentInstance.navItems().map((item) => item.action)
    expect(actions).not.toContain('organization')
  })

  describe('Administration submenu', () => {
    function setupOnAdminRoutes() {
      TestBed.resetTestingModule()
      TestBed.configureTestingModule({
        imports: [AppShell],
        providers: [
          provideHttpClient(),
          provideHttpClientTesting(),
          ...authProviders,
          ...userProviders,
          ...repositoryProviders,
          ...meProviders,
          ...versionProviders,
          ...readableCatalogProviders,
          provideRouter([
            { path: 'admin', component: DummyRoutedComponent },
            { path: 'admin/export', component: DummyRoutedComponent },
          ]),
        ],
      })
      httpMock = TestBed.inject(HttpTestingController)
      return TestBed.createComponent(AppShell)
    }

    function adminItem(fixture: ReturnType<typeof TestBed.createComponent<AppShell>>) {
      return fixture.componentInstance.navItems().find((item) => item.action === 'admin')!
    }

    it('is collapsed by default and does not render its children', () => {
      const fixture = TestBed.createComponent(AppShell)
      fixture.detectChanges()
      flushMe({ id: 'user-1', username: 'florian', is_super_admin: true })
      fixture.detectChanges()

      expect(fixture.componentInstance.isMenuOpen(adminItem(fixture))).toBe(false)
      expect(fixture.nativeElement.querySelector('.app-shell__nav-submenu')).toBeFalsy()
    })

    it('opens automatically, marks the child active (not the parent), when landing on a sub-page', async () => {
      const fixture = setupOnAdminRoutes()
      fixture.detectChanges()
      flushMe({ id: 'user-1', username: 'florian', is_super_admin: true })
      fixture.detectChanges()

      const router = TestBed.inject(Router)
      await router.navigateByUrl('/admin/export')
      fixture.detectChanges()
      await fixture.whenStable()
      fixture.detectChanges()

      expect(fixture.componentInstance.isMenuOpen(adminItem(fixture))).toBe(true)

      const adminLink: HTMLAnchorElement = fixture.nativeElement.querySelector(
        '.app-shell__nav-group a.app-shell__nav-link',
      )
      const links: HTMLAnchorElement[] = Array.from(
        fixture.nativeElement.querySelectorAll('a.app-shell__nav-link--sub'),
      )
      const exportLink = links.find((el) => el.textContent?.includes('Export'))!

      expect(adminLink.classList.contains('app-shell__nav-link--active')).toBe(false)
      expect(exportLink).toBeTruthy()
      expect(exportLink.classList.contains('app-shell__nav-link--active')).toBe(true)
      expect(adminLink.getAttribute('aria-current')).toBeNull()
      expect(exportLink.getAttribute('aria-current')).toBe('page')
    })

    it('toggles open and closed via the chevron button, independent of route', () => {
      const fixture = TestBed.createComponent(AppShell)
      fixture.detectChanges()
      flushMe({ id: 'user-1', username: 'florian', is_super_admin: true })
      fixture.detectChanges()

      const toggle: HTMLButtonElement = fixture.nativeElement.querySelector(
        '.app-shell__nav-group-toggle',
      )
      toggle.click()
      fixture.detectChanges()
      expect(fixture.componentInstance.isMenuOpen(adminItem(fixture))).toBe(true)
      expect(fixture.nativeElement.querySelector('.app-shell__nav-submenu')).toBeTruthy()

      toggle.click()
      fixture.detectChanges()
      expect(fixture.componentInstance.isMenuOpen(adminItem(fixture))).toBe(false)
      expect(fixture.nativeElement.querySelector('.app-shell__nav-submenu')).toBeFalsy()
    })

    it('can be manually collapsed even while on one of its own sub-pages', async () => {
      const fixture = setupOnAdminRoutes()
      fixture.detectChanges()
      flushMe({ id: 'user-1', username: 'florian', is_super_admin: true })
      fixture.detectChanges()
      const router = TestBed.inject(Router)
      await router.navigateByUrl('/admin/export')
      fixture.detectChanges()
      expect(fixture.componentInstance.isMenuOpen(adminItem(fixture))).toBe(true)

      fixture.nativeElement.querySelector('.app-shell__nav-group-toggle').click()
      fixture.detectChanges()

      expect(fixture.componentInstance.isMenuOpen(adminItem(fixture))).toBe(false)
    })
  })

  describe('page title', () => {
    function setupWithTitledRoutes() {
      TestBed.resetTestingModule()
      TestBed.configureTestingModule({
        imports: [AppShell],
        providers: [
          provideHttpClient(),
          provideHttpClientTesting(),
          ...authProviders,
          ...userProviders,
          ...repositoryProviders,
          ...meProviders,
          ...versionProviders,
          ...readableCatalogProviders,
          provideRouter([
            {
              path: 'repositories',
              component: DummyRoutedComponent,
              data: { titleKey: 'nav.repositories' },
            },
            {
              path: 'admin',
              component: DummyRoutedComponent,
              data: { titleKey: 'nav.administration' },
            },
          ]),
        ],
      })
      httpMock = TestBed.inject(HttpTestingController)
      return TestBed.createComponent(AppShell)
    }

    it("reads the initial route's static title on first render", async () => {
      const fixture = setupWithTitledRoutes()
      const router = TestBed.inject(Router)
      await router.navigateByUrl('/repositories')
      fixture.detectChanges()
      flushMe({ id: 'user-1', username: 'florian', is_super_admin: false })
      fixture.detectChanges()

      expect(fixture.componentInstance.pageTitle.title()).toBe('Dépôts')
      expect(fixture.nativeElement.querySelector('.app-shell__page-title').textContent).toContain(
        'Dépôts',
      )
    })

    it('translates the title again and re-creates the routed view on a language change', async () => {
      const fixture = setupWithTitledRoutes()
      const router = TestBed.inject(Router)
      await router.navigateByUrl('/repositories')
      fixture.detectChanges()
      flushMe({ id: 'user-1', username: 'florian', is_super_admin: false })
      fixture.detectChanges()
      const generation = fixture.componentInstance.viewGeneration()

      registerTranslator((key) => `en:${key}`)
      await TestBed.inject(LanguageService).use('en')
      fixture.detectChanges()

      expect(fixture.componentInstance.pageTitle.title()).toBe('en:nav.repositories')
      expect(fixture.componentInstance.viewGeneration()).toBe(generation + 1)
    })

    it("updates to the new route's title on navigation", async () => {
      const fixture = setupWithTitledRoutes()
      const router = TestBed.inject(Router)
      await router.navigateByUrl('/repositories')
      fixture.detectChanges()
      flushMe({ id: 'user-1', username: 'florian', is_super_admin: false })
      fixture.detectChanges()
      expect(fixture.componentInstance.pageTitle.title()).toBe('Dépôts')

      await router.navigateByUrl('/admin')
      fixture.detectChanges()

      expect(fixture.componentInstance.pageTitle.title()).toBe('Administration')
    })
  })

  it('clears the session and routes to /login when logout() is called', () => {
    const auth = TestBed.inject(AuthService)
    const router = TestBed.inject(Router)
    const navigate = vi.spyOn(router, 'navigateByUrl').mockResolvedValue(true)
    sessionStorage.setItem('artiferris_token', 'a-valid-jwt')
    auth.token.set('a-valid-jwt')

    const fixture = TestBed.createComponent(AppShell)
    fixture.detectChanges()
    flushMe({ id: 'user-1', username: 'florian', is_super_admin: false })

    fixture.componentInstance.logout()

    expect(auth.token()).toBeNull()
    expect(sessionStorage.getItem('artiferris_token')).toBeNull()
    expect(navigate).toHaveBeenCalledWith('/login')
  })

  describe('when /api/me fails', () => {
    function failMe(status: number) {
      httpMock.expectOne('/api/version').flush({ version: '0.2.3' })
      httpMock.expectOne('/api/me').flush({}, { status, statusText: 'Error' })
    }

    it.each([500, 502, 429, 0])(
      'keeps the session and offers a retry on a transient failure (%i)',
      (status) => {
        const auth = TestBed.inject(AuthService)
        const navigate = vi.spyOn(TestBed.inject(Router), 'navigateByUrl').mockResolvedValue(true)
        auth.token.set('a-valid-jwt')

        const fixture = TestBed.createComponent(AppShell)
        fixture.detectChanges()
        failMe(status)
        fixture.detectChanges()

        expect(auth.token()).toBe('a-valid-jwt')
        expect(navigate).not.toHaveBeenCalled()
        expect(fixture.nativeElement.querySelector('[role="alert"]')).toBeTruthy()
        expect(fixture.nativeElement.textContent).toContain('Réessayer')
        expect(fixture.nativeElement.querySelector('gbt-app-shell')).toBeNull()
      },
    )

    it('loads the shell after the retry succeeds', () => {
      const auth = TestBed.inject(AuthService)
      auth.token.set('a-valid-jwt')
      const fixture = TestBed.createComponent(AppShell)
      fixture.detectChanges()
      failMe(503)
      fixture.detectChanges()

      const retry: HTMLButtonElement = Array.from<HTMLButtonElement>(
        fixture.nativeElement.querySelectorAll('button'),
      ).find((button) => button.textContent?.includes('Réessayer'))!
      retry.click()
      fixture.detectChanges()
      httpMock.expectOne('/api/me').flush({
        id: 'u',
        username: 'florian',
        is_super_admin: false,
        is_organization_admin: false,
      })
      httpMock.expectOne('/api/repositories').flush([])
      fixture.detectChanges()

      expect(fixture.nativeElement.querySelector('[role="alert"]')).toBeNull()
      expect(fixture.nativeElement.textContent).toContain('florian')
      expect(auth.token()).toBe('a-valid-jwt')
    })

    it.each([401, 403])('ends the session and goes to /login on a %i', (status) => {
      const auth = TestBed.inject(AuthService)
      const navigate = vi.spyOn(TestBed.inject(Router), 'navigateByUrl').mockResolvedValue(true)
      auth.token.set('a-valid-jwt')

      const fixture = TestBed.createComponent(AppShell)
      fixture.detectChanges()
      failMe(status)

      expect(auth.token()).toBeNull()
      expect(navigate).toHaveBeenCalledWith('/login')
    })
  })

  it('toggles the user menu open and closed when the username is clicked', () => {
    const fixture = TestBed.createComponent(AppShell)
    fixture.detectChanges()
    flushMe({ id: 'user-1', username: 'florian', is_super_admin: false })
    fixture.detectChanges()

    const trigger: HTMLButtonElement = fixture.nativeElement.querySelector(
      '.app-shell__user-trigger',
    )
    trigger.click()
    fixture.detectChanges()
    expect(fixture.nativeElement.querySelector('.app-shell__user-dropdown')).toBeTruthy()

    trigger.click()
    fixture.detectChanges()
    expect(fixture.nativeElement.querySelector('.app-shell__user-dropdown')).toBeFalsy()
  })

  it('closes the user menu when clicking outside it', () => {
    const fixture = TestBed.createComponent(AppShell)
    fixture.detectChanges()
    flushMe({ id: 'user-1', username: 'florian', is_super_admin: false })
    fixture.detectChanges()

    fixture.nativeElement.querySelector('.app-shell__user-trigger').click()
    fixture.detectChanges()
    expect(fixture.componentInstance.userMenuOpen()).toBe(true)

    document.body.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    fixture.detectChanges()
    expect(fixture.componentInstance.userMenuOpen()).toBe(false)
  })

  it('closes the user menu on Escape', () => {
    const fixture = TestBed.createComponent(AppShell)
    fixture.detectChanges()
    flushMe({ id: 'user-1', username: 'florian', is_super_admin: false })
    fixture.detectChanges()

    fixture.componentInstance.userMenuOpen.set(true)
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))

    expect(fixture.componentInstance.userMenuOpen()).toBe(false)
  })

  it('the user menu links to /account and closes on click', () => {
    TestBed.resetTestingModule()
    TestBed.configureTestingModule({
      imports: [AppShell],
      providers: [
        provideHttpClient(),
        provideHttpClientTesting(),
        ...authProviders,
        ...userProviders,
        ...repositoryProviders,
        ...meProviders,
        ...versionProviders,
        ...readableCatalogProviders,
        provideRouter([{ path: 'account', component: DummyRoutedComponent }]),
      ],
    })
    httpMock = TestBed.inject(HttpTestingController)

    const fixture = TestBed.createComponent(AppShell)
    fixture.detectChanges()
    flushMe({ id: 'user-1', username: 'florian', is_super_admin: false })
    fixture.detectChanges()
    fixture.componentInstance.userMenuOpen.set(true)
    fixture.detectChanges()

    const accountLink: HTMLAnchorElement = fixture.nativeElement.querySelector(
      '.app-shell__user-dropdown-item[href="/account"]',
    )
    expect(accountLink).toBeTruthy()
    expect(accountLink.textContent).toContain('Mon compte')

    accountLink.click()
    fixture.detectChanges()
    expect(fixture.componentInstance.userMenuOpen()).toBe(false)
  })

  describe('search', () => {
    it('filters repositories (and users, for super-admins) by the typed query', () => {
      const fixture = TestBed.createComponent(AppShell)
      fixture.detectChanges()
      httpMock.expectOne('/api/version').flush({ version: '0.2.3' })
      httpMock
        .expectOne('/api/me')
        .flush({ id: 'user-1', username: 'florian', is_super_admin: true })
      httpMock.expectOne('/api/repositories').flush([
        {
          id: 'r1',
          name: 'my-repo',
          format: 'npm',
          repo_type: 'hosted',
          remote_url: null,
          group_members: [],
        },
        {
          id: 'r2',
          name: 'other',
          format: 'docker',
          repo_type: 'hosted',
          remote_url: null,
          group_members: [],
        },
      ])
      httpMock.expectOne('/api/users').flush([
        { id: 'u1', username: 'my-user', is_super_admin: false },
        { id: 'u2', username: 'someone-else', is_super_admin: false },
      ])
      fixture.detectChanges()

      fixture.componentInstance.onSearchInput('my')
      // Starting a search re-fetches, so the header search never shows stale data.
      httpMock.expectOne('/api/repositories').flush([
        {
          id: 'r1',
          name: 'my-repo',
          format: 'npm',
          repo_type: 'hosted',
          remote_url: null,
          group_members: [],
        },
      ])
      httpMock
        .expectOne('/api/users')
        .flush([{ id: 'u1', username: 'my-user', is_super_admin: false }])

      const categories = fixture.componentInstance.searchResults()
      expect(categories.find((c) => c.label === 'Dépôts')?.items).toEqual([
        { kind: 'repository', id: 'r1', label: 'my-repo' },
      ])
      expect(categories.find((c) => c.label === 'Utilisateurs')?.items).toEqual([
        { kind: 'user', id: 'u1', label: 'my-user' },
      ])
    })

    it('navigates to the selected result and clears the query', () => {
      const fixture = TestBed.createComponent(AppShell)
      fixture.detectChanges()
      httpMock.expectOne('/api/version').flush({ version: '0.2.3' })
      httpMock
        .expectOne('/api/me')
        .flush({ id: 'user-1', username: 'florian', is_super_admin: false })
      httpMock.expectOne('/api/repositories').flush([
        {
          id: 'r1',
          name: 'my-repo',
          format: 'npm',
          repo_type: 'hosted',
          remote_url: null,
          group_members: [],
        },
      ])
      fixture.detectChanges()

      const router = TestBed.inject(Router)
      const navigate = vi.spyOn(router, 'navigateByUrl').mockResolvedValue(true)
      fixture.componentInstance.onSearchInput('my')
      httpMock.expectOne('/api/repositories').flush([
        {
          id: 'r1',
          name: 'my-repo',
          format: 'npm',
          repo_type: 'hosted',
          remote_url: null,
          group_members: [],
        },
      ])
      fixture.componentInstance.onSelectResult({ kind: 'repository', id: 'r1', label: 'my-repo' })

      expect(navigate).toHaveBeenCalledWith('/repositories/r1')
      expect(fixture.componentInstance.searchQuery()).toBe('')
    })

    it('re-fetches repositories when a search starts, so a repo created elsewhere in the session is found', () => {
      const fixture = TestBed.createComponent(AppShell)
      fixture.detectChanges()
      httpMock.expectOne('/api/version').flush({ version: '0.2.3' })
      httpMock
        .expectOne('/api/me')
        .flush({ id: 'user-1', username: 'florian', is_super_admin: false })
      httpMock.expectOne('/api/repositories').flush([])
      fixture.detectChanges()

      fixture.componentInstance.onSearchInput('new')
      httpMock.expectOne('/api/repositories').flush([
        {
          id: 'r1',
          name: 'newly-created',
          format: 'npm',
          repo_type: 'hosted',
          remote_url: null,
          group_members: [],
        },
      ])

      const categories = fixture.componentInstance.searchResults()
      expect(categories.find((c) => c.label === 'Dépôts')?.items).toEqual([
        { kind: 'repository', id: 'r1', label: 'newly-created' },
      ])
    })

    it('does not re-fetch on every keystroke of the same search', () => {
      const fixture = TestBed.createComponent(AppShell)
      fixture.detectChanges()
      httpMock.expectOne('/api/version').flush({ version: '0.2.3' })
      httpMock
        .expectOne('/api/me')
        .flush({ id: 'user-1', username: 'florian', is_super_admin: false })
      httpMock.expectOne('/api/repositories').flush([])
      fixture.detectChanges()

      fixture.componentInstance.onSearchInput('m')
      httpMock.expectOne('/api/repositories').flush([])
      fixture.componentInstance.onSearchInput('my')

      httpMock.verify()
    })

    it('flags a failed search load, says so in the results panel, and retries on the next keystroke', () => {
      const fixture = TestBed.createComponent(AppShell)
      fixture.detectChanges()
      httpMock.expectOne('/api/version').flush({ version: '0.2.3' })
      httpMock
        .expectOne('/api/me')
        .flush({ id: 'user-1', username: 'florian', is_super_admin: false })
      httpMock.expectOne('/api/repositories').flush([])
      fixture.detectChanges()

      const input: HTMLInputElement = fixture.nativeElement.querySelector('gbt-search-bar input')
      input.value = 'm'
      input.dispatchEvent(new Event('input'))
      httpMock
        .expectOne('/api/repositories')
        .flush(null, { status: 500, statusText: 'Server Error' })
      fixture.detectChanges()

      expect(fixture.componentInstance.searchFailed()).toBe(true)
      expect(fixture.nativeElement.querySelector('gbt-search-bar').textContent).toContain(
        'La recherche a échoué',
      )

      fixture.componentInstance.onSearchInput('my')
      httpMock.expectOne('/api/repositories').flush([
        {
          id: 'r1',
          name: 'my-repo',
          format: 'npm',
          repo_type: 'hosted',
          remote_url: null,
          group_members: [],
        },
      ])
      expect(fixture.componentInstance.searchFailed()).toBe(false)
      expect(
        fixture.componentInstance.searchResults().find((c) => c.label === 'Dépôts')?.items,
      ).toHaveLength(1)
    })

    it('re-fetches again on a new search after the previous one was cleared', () => {
      const fixture = TestBed.createComponent(AppShell)
      fixture.detectChanges()
      httpMock.expectOne('/api/version').flush({ version: '0.2.3' })
      httpMock
        .expectOne('/api/me')
        .flush({ id: 'user-1', username: 'florian', is_super_admin: false })
      httpMock.expectOne('/api/repositories').flush([])
      fixture.detectChanges()

      fixture.componentInstance.onSearchInput('first')
      httpMock.expectOne('/api/repositories').flush([])
      // clearing must reset the "new search" state, or the next search reuses the cache
      fixture.componentInstance.onSearchInput('')
      fixture.componentInstance.onSearchInput('second')

      httpMock.expectOne('/api/repositories').flush([])
    })
  })

  describe('package search', () => {
    const PACKAGE_DEBOUNCE_MS = 250

    beforeEach(() => vi.useFakeTimers())
    afterEach(() => vi.useRealTimers())

    function setup(me: { is_super_admin: boolean; is_organization_admin?: boolean }) {
      const fixture = TestBed.createComponent(AppShell)
      fixture.detectChanges()
      flushMe({ id: 'user-1', username: 'florian', ...me })
      fixture.detectChanges()
      return fixture
    }

    function search(fixture: ReturnType<typeof setup>, query: string) {
      fixture.componentInstance.onSearchInput(query)
      for (const req of httpMock.match('/api/repositories')) {
        req.flush([])
      }
      for (const req of httpMock.match('/api/users')) {
        req.flush([])
      }
    }

    const packageRequests = () => httpMock.match((req) => req.url === '/api/search')
    const packageCategory = (fixture: ReturnType<typeof setup>) =>
      fixture.componentInstance.searchResults().find((c) => c.label === 'Paquets et images')

    it.each(['', ' ', 'a', ' a '])('does not query below two characters: %j', (query) => {
      const fixture = setup({ is_super_admin: false })
      search(fixture, query)
      vi.advanceTimersByTime(PACKAGE_DEBOUNCE_MS * 4)

      expect(packageRequests()).toEqual([])
    })

    it('waits for a pause in typing, then sends a single request for the last text', () => {
      const fixture = setup({ is_super_admin: false })
      search(fixture, 'le')
      vi.advanceTimersByTime(PACKAGE_DEBOUNCE_MS - 50)
      search(fixture, 'lef')
      vi.advanceTimersByTime(PACKAGE_DEBOUNCE_MS - 1)
      expect(packageRequests()).toEqual([])

      vi.advanceTimersByTime(1)
      const requests = packageRequests()
      expect(requests).toHaveLength(1)
      expect(requests[0].request.method).toBe('GET')
      expect(requests[0].request.params.get('q')).toBe('lef')
      expect(requests[0].request.params.get('per_page')).toBe('5')
      expect(requests[0].request.params.keys()).toEqual(['q', 'per_page'])
    })

    it('trims the text before sending it', () => {
      const fixture = setup({ is_super_admin: false })
      search(fixture, '  left ')
      vi.advanceTimersByTime(PACKAGE_DEBOUNCE_MS)

      expect(packageRequests()[0].request.params.get('q')).toBe('left')
    })

    it('lists packages and images under "Paquets et images", after the other categories', () => {
      const fixture = setup({ is_super_admin: true })
      search(fixture, 'a')
      search(fixture, 'ap')
      vi.advanceTimersByTime(PACKAGE_DEBOUNCE_MS)
      packageRequests()[0].flush(
        readableSearchResult([readableEntry(), readableDockerEntry(), proxiedEntry()]),
      )

      const categories = fixture.componentInstance.searchResults()
      expect(categories.map((c) => c.label)).toEqual([
        'Dépôts',
        'Utilisateurs',
        'Paquets et images',
      ])
      expect(categories[2].items).toEqual([
        { kind: 'package', id: 'r-npm', format: 'npm', name: 'left-pad', label: 'left-pad (npm)' },
        { kind: 'package', id: 'r-docker', format: 'docker', name: 'api', label: 'api (Docker)' },
        {
          kind: 'package',
          id: 'r-proxy',
          format: 'npm',
          name: 'lodash',
          label: 'lodash (npm) — cache du proxy',
        },
      ])
    })

    it('searches packages for a member who sees neither users nor admin items', () => {
      const fixture = setup({ is_super_admin: false })
      search(fixture, 'left')
      vi.advanceTimersByTime(PACKAGE_DEBOUNCE_MS)
      packageRequests()[0].flush(readableSearchResult([readableEntry()]))

      expect(fixture.componentInstance.searchResults().map((c) => c.label)).toEqual([
        'Dépôts',
        'Paquets et images',
      ])
    })

    it('shows no category when nothing matches', () => {
      const fixture = setup({ is_super_admin: false })
      search(fixture, 'zzz')
      vi.advanceTimersByTime(PACKAGE_DEBOUNCE_MS)
      packageRequests()[0].flush(readableSearchResult([]))

      expect(packageCategory(fixture)).toBeUndefined()
    })

    it('flags a failed package search', () => {
      const fixture = setup({ is_super_admin: false })
      search(fixture, 'zzz')
      vi.advanceTimersByTime(PACKAGE_DEBOUNCE_MS)
      packageRequests()[0].flush(null, { status: 500, statusText: 'Server Error' })

      expect(fixture.componentInstance.searchFailed()).toBe(true)
      expect(packageCategory(fixture)).toBeUndefined()
    })

    it('renders the packages in the search bar', () => {
      const fixture = setup({ is_super_admin: false })
      const input: HTMLInputElement = fixture.nativeElement.querySelector('gbt-search-bar input')
      input.value = 'lod'
      input.dispatchEvent(new Event('input'))
      httpMock.match('/api/repositories').forEach((req) => req.flush([]))
      vi.advanceTimersByTime(PACKAGE_DEBOUNCE_MS)
      packageRequests()[0].flush(readableSearchResult([proxiedEntry()]))
      fixture.detectChanges()

      const text = fixture.nativeElement.querySelector('gbt-search-bar').textContent
      expect(text).toContain('Paquets et images')
      expect(text).toContain('lodash (npm) — cache du proxy')
    })

    it('navigates to the package page of the selected result and clears the search', () => {
      const fixture = setup({ is_super_admin: false })
      const navigate = vi.spyOn(TestBed.inject(Router), 'navigate').mockResolvedValue(true)
      search(fixture, 'api')
      vi.advanceTimersByTime(PACKAGE_DEBOUNCE_MS)
      packageRequests()[0].flush(readableSearchResult([readableDockerEntry({ name: 'team/api' })]))

      const item = packageCategory(fixture)!.items[0]
      fixture.componentInstance.onSelectResult(item as never)

      expect(navigate).toHaveBeenCalledWith([
        '/repositories',
        'r-docker',
        'packages',
        'docker',
        'team/api',
      ])
      const router = TestBed.inject(Router)
      expect(router.serializeUrl(router.createUrlTree(navigate.mock.calls[0][0]))).toBe(
        '/repositories/r-docker/packages/docker/team%2Fapi',
      )
      expect(fixture.componentInstance.searchQuery()).toBe('')
      expect(packageCategory(fixture)).toBeUndefined()
    })

    it('cancels the pending request and drops the category as soon as the text gets too short', () => {
      const fixture = setup({ is_super_admin: false })
      search(fixture, 'left')
      vi.advanceTimersByTime(PACKAGE_DEBOUNCE_MS)
      packageRequests()[0].flush(readableSearchResult([readableEntry()]))
      expect(packageCategory(fixture)).toBeDefined()

      search(fixture, 'lef')
      vi.advanceTimersByTime(PACKAGE_DEBOUNCE_MS)
      const inFlight = packageRequests()[0]
      search(fixture, 'l')

      expect(inFlight.cancelled).toBe(true)
      expect(packageCategory(fixture)).toBeUndefined()
    })

    it('never lets a slower, older response overwrite the newer one', () => {
      const fixture = setup({ is_super_admin: false })
      search(fixture, 'ab')
      vi.advanceTimersByTime(PACKAGE_DEBOUNCE_MS)
      const [older] = packageRequests()
      search(fixture, 'abc')
      vi.advanceTimersByTime(PACKAGE_DEBOUNCE_MS)
      const [newer] = packageRequests()

      expect(older.cancelled).toBe(true)
      newer.flush(readableSearchResult([readableEntry({ name: 'abc-new' })]))
      expect(packageCategory(fixture)?.items).toHaveLength(1)
      expect(packageCategory(fixture)?.items[0]).toMatchObject({ name: 'abc-new' })
    })

    it('ignores a response that lands after the search was cleared', () => {
      const fixture = setup({ is_super_admin: false })
      search(fixture, 'left')
      vi.advanceTimersByTime(PACKAGE_DEBOUNCE_MS)
      const [inFlight] = packageRequests()
      search(fixture, '')

      expect(inFlight.cancelled).toBe(true)
      expect(packageCategory(fixture)).toBeUndefined()
    })

    it.each([400, 401, 429, 500])(
      'silently shows no category when the request fails with %i',
      (status) => {
        const fixture = setup({ is_super_admin: false })
        search(fixture, 'left')
        vi.advanceTimersByTime(PACKAGE_DEBOUNCE_MS)
        packageRequests()[0].flush({ error: 'nope' }, { status, statusText: 'Error' })

        expect(packageCategory(fixture)).toBeUndefined()
        expect(fixture.componentInstance.searchResults().map((c) => c.label)).toEqual(['Dépôts'])
      },
    )

    it('drops earlier results when a later request fails, and recovers on the next one', () => {
      const fixture = setup({ is_super_admin: false })
      search(fixture, 'left')
      vi.advanceTimersByTime(PACKAGE_DEBOUNCE_MS)
      packageRequests()[0].flush(readableSearchResult([readableEntry()]))
      search(fixture, 'left-')
      vi.advanceTimersByTime(PACKAGE_DEBOUNCE_MS)
      packageRequests()[0].flush({}, { status: 429, statusText: 'Too Many Requests' })
      expect(packageCategory(fixture)).toBeUndefined()

      search(fixture, 'left-p')
      vi.advanceTimersByTime(PACKAGE_DEBOUNCE_MS)
      packageRequests()[0].flush(readableSearchResult([readableEntry()]))
      expect(packageCategory(fixture)?.items).toHaveLength(1)
    })

    it('keeps filtering repositories and users when the package search fails', () => {
      const fixture = setup({ is_super_admin: true })
      fixture.componentInstance.onSearchInput('my')
      httpMock.expectOne('/api/repositories').flush([
        {
          id: 'r1',
          name: 'my-repo',
          format: 'npm',
          repo_type: 'hosted',
          remote_url: null,
          group_members: [],
        },
      ])
      httpMock
        .expectOne('/api/users')
        .flush([{ id: 'u1', username: 'my-user', is_super_admin: false }])
      vi.advanceTimersByTime(PACKAGE_DEBOUNCE_MS)
      packageRequests()[0].flush({}, { status: 500, statusText: 'Error' })

      const categories = fixture.componentInstance.searchResults()
      expect(categories.map((c) => c.label)).toEqual(['Dépôts', 'Utilisateurs'])
      expect(categories[0].items).toHaveLength(1)
      expect(categories[1].items).toHaveLength(1)
    })

    it('does not request the same text twice in a row', () => {
      const fixture = setup({ is_super_admin: false })
      search(fixture, 'left')
      vi.advanceTimersByTime(PACKAGE_DEBOUNCE_MS)
      packageRequests()[0].flush(readableSearchResult([]))
      search(fixture, 'left ')
      vi.advanceTimersByTime(PACKAGE_DEBOUNCE_MS)

      expect(packageRequests()).toEqual([])
    })

    it('searches again for the same text after the search was cleared', () => {
      const fixture = setup({ is_super_admin: false })
      search(fixture, 'left')
      vi.advanceTimersByTime(PACKAGE_DEBOUNCE_MS)
      packageRequests()[0].flush(readableSearchResult([]))
      search(fixture, '')
      search(fixture, 'left')
      vi.advanceTimersByTime(PACKAGE_DEBOUNCE_MS)

      expect(packageRequests()).toHaveLength(1)
    })
  })

  it('logs out via the user menu\'s "Déconnexion" item', () => {
    const auth = TestBed.inject(AuthService)
    const router = TestBed.inject(Router)
    const navigate = vi.spyOn(router, 'navigateByUrl').mockResolvedValue(true)
    sessionStorage.setItem('artiferris_token', 'a-valid-jwt')
    auth.token.set('a-valid-jwt')

    const fixture = TestBed.createComponent(AppShell)
    fixture.detectChanges()
    flushMe({ id: 'user-1', username: 'florian', is_super_admin: false })
    fixture.detectChanges()
    fixture.componentInstance.userMenuOpen.set(true)
    fixture.detectChanges()

    const items: HTMLButtonElement[] = Array.from(
      fixture.nativeElement.querySelectorAll('button.app-shell__user-dropdown-item'),
    )
    const logoutItem = items.find((el) => el.textContent?.includes('Déconnexion'))!
    logoutItem.click()

    expect(auth.token()).toBeNull()
    expect(navigate).toHaveBeenCalledWith('/login')
    expect(fixture.componentInstance.userMenuOpen()).toBe(false)
  })
})
