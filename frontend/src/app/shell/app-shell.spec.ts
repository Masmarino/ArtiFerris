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
    expect(actions).toEqual(['explorer', 'repositories', 'my-repository', 'admin'])

    // As in FerrisGit: the dashboard first, the instance's configuration last.
    const adminItem = fixture.componentInstance.navItems().find((item) => item.action === 'admin')!
    expect(adminItem.children?.map((child) => child.action)).toEqual([
      'dashboard',
      'users',
      'organizations',
      'health',
      'export',
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
      fixture.nativeElement.querySelectorAll('a.gbt-app-shell__link'),
    )
    const explorerLink = links.find((el) => el.textContent?.includes('Explorer'))!
    const repositoriesLink = links.find((el) => el.textContent?.includes('Dépôts'))!

    expect(explorerLink.getAttribute('aria-current')).toBeNull()
    expect(repositoriesLink.getAttribute('aria-current')).toBe('page')
  })

  it('puts Utilisateurs under Administration for an organization admin, even though they are not a super-admin', () => {
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

    const admin = fixture.componentInstance.navItems().find((item) => item.action === 'admin')
    expect(admin?.children?.map((child) => child.action)).toContain('users')
  })

  it("gives an org-admin who is not a super-admin an Administration group: its users, then its own organization's settings", () => {
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
    expect(actions).toEqual(['explorer', 'repositories', 'my-repository', 'admin'])
    const admin = fixture.componentInstance.navItems().find((item) => item.action === 'admin')!
    expect(admin.text).toBe('Administration')
    expect(admin.children?.map((child) => [child.text, child.link])).toEqual([
      ['Utilisateurs', '/users'],
      ['Réglages', '/admin/organizations/org-1'],
    ])
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
            { path: 'users', component: DummyRoutedComponent },
          ]),
        ],
      })
      httpMock = TestBed.inject(HttpTestingController)
      return TestBed.createComponent(AppShell)
    }

    function adminItem(fixture: ReturnType<typeof TestBed.createComponent<AppShell>>) {
      return fixture.componentInstance.navItems().find((item) => item.action === 'admin')!
    }

    function panel(fixture: ReturnType<typeof TestBed.createComponent<AppShell>>): HTMLElement {
      return fixture.nativeElement.querySelector('.gbt-app-shell-nav-group__panel')
    }

    it('is collapsed by default and does not render its children', () => {
      const fixture = TestBed.createComponent(AppShell)
      fixture.detectChanges()
      flushMe({ id: 'user-1', username: 'florian', is_super_admin: true })
      fixture.detectChanges()

      expect(fixture.componentInstance.isMenuOpen(adminItem(fixture))).toBe(false)
      expect(panel(fixture).hidden).toBe(true)
    })

    it('opens on the users page too, which lives outside /admin', async () => {
      const fixture = setupOnAdminRoutes()
      fixture.detectChanges()
      flushMe({ id: 'user-1', username: 'florian', is_super_admin: true })
      fixture.detectChanges()

      await TestBed.inject(Router).navigateByUrl('/users')
      fixture.detectChanges()

      expect(fixture.componentInstance.isMenuOpen(adminItem(fixture))).toBe(true)
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

      const links: HTMLAnchorElement[] = Array.from(panel(fixture).querySelectorAll('a'))
      const dashboardLink = links.find((el) => el.textContent?.includes('Tableau de bord'))!
      const exportLink = links.find((el) => el.textContent?.includes('Export'))!

      expect(panel(fixture).hidden).toBe(false)
      expect(dashboardLink.getAttribute('aria-current')).toBeNull()
      expect(exportLink.getAttribute('aria-current')).toBe('page')
    })

    it('toggles open and closed with the group button, independent of route', () => {
      const fixture = TestBed.createComponent(AppShell)
      fixture.detectChanges()
      flushMe({ id: 'user-1', username: 'florian', is_super_admin: true })
      fixture.detectChanges()

      const toggle: HTMLButtonElement = fixture.nativeElement.querySelector(
        '.gbt-app-shell-nav-group__toggle',
      )
      expect(toggle.textContent).toContain('Administration')
      toggle.click()
      fixture.detectChanges()
      expect(fixture.componentInstance.isMenuOpen(adminItem(fixture))).toBe(true)
      expect(panel(fixture).hidden).toBe(false)

      toggle.click()
      fixture.detectChanges()
      expect(fixture.componentInstance.isMenuOpen(adminItem(fixture))).toBe(false)
      expect(panel(fixture).hidden).toBe(true)
    })

    it('opens on the dashboard first, as the group button no longer links to it', () => {
      const fixture = TestBed.createComponent(AppShell)
      fixture.detectChanges()
      flushMe({ id: 'user-1', username: 'florian', is_super_admin: true })
      fixture.detectChanges()

      const first = panel(fixture).querySelector('a')!
      expect(first.textContent).toContain('Tableau de bord')
      expect(first.getAttribute('href')).toBe('/admin')
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

      fixture.nativeElement.querySelector('.gbt-app-shell-nav-group__toggle').click()
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
              data: { titleKey: 'nav.dashboard', trail: [{ labelKey: 'nav.administration' }] },
            },
            {
              path: 'admin/export',
              component: DummyRoutedComponent,
              data: {
                titleKey: 'nav.export',
                trail: [{ labelKey: 'nav.administration', link: '/admin' }],
              },
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
      // The bar's breadcrumb ends on the page, for assistive technology; the page shows its own h1.
      expect(
        fixture.nativeElement.querySelector('.app-shell__breadcrumb [aria-current="page"]')
          .textContent,
      ).toContain('Dépôts')
      expect(fixture.nativeElement.querySelector('.app-shell__header h1')).toBeNull()
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

    it("shows what sits above the page in the bar's breadcrumb, from the route's trail", async () => {
      const fixture = setupWithTitledRoutes()
      const router = TestBed.inject(Router)
      await router.navigateByUrl('/admin/export')
      fixture.detectChanges()
      flushMe({ id: 'user-1', username: 'florian', is_super_admin: true })
      fixture.detectChanges()

      const steps = Array.from(
        fixture.nativeElement.querySelectorAll('.app-shell__breadcrumb a'),
      ) as HTMLAnchorElement[]
      expect(steps.map((a) => a.textContent?.trim())).toEqual(['Administration'])
      expect(steps[0].getAttribute('href')).toBe('/admin')

      await router.navigateByUrl('/repositories')
      fixture.detectChanges()
      expect(fixture.nativeElement.querySelector('.app-shell__breadcrumb a')).toBeNull()
    })

    it('names a section with no page of its own without making it a link', async () => {
      const fixture = setupWithTitledRoutes()
      const router = TestBed.inject(Router)
      await router.navigateByUrl('/admin')
      fixture.detectChanges()
      flushMe({ id: 'user-1', username: 'florian', is_super_admin: true })
      fixture.detectChanges()

      const breadcrumb = fixture.nativeElement.querySelector('.app-shell__breadcrumb')
      expect(breadcrumb.querySelector('a')).toBeNull()
      expect(breadcrumb.querySelector('.app-shell__crumb')?.textContent?.trim()).toBe(
        'Administration',
      )
      expect(fixture.componentInstance.pageTitle.title()).toBe('Tableau de bord')
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

      expect(fixture.componentInstance.pageTitle.title()).toBe('Tableau de bord')
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

  /** Opens Gabarit's menu, labelled with the username, as a click does. */
  function openUserMenu(fixture: { nativeElement: HTMLElement; detectChanges(): void }) {
    const trigger = fixture.nativeElement.querySelector<HTMLButtonElement>(
      '.app-shell__header-end .gbt-menu__trigger',
    )!
    trigger.click()
    fixture.detectChanges()
    return trigger
  }

  it('names the user menu after the user, and lists the account, the docs and signing out', () => {
    const fixture = TestBed.createComponent(AppShell)
    fixture.detectChanges()
    flushMe({ id: 'user-1', username: 'florian', is_super_admin: false })
    fixture.detectChanges()

    const trigger = openUserMenu(fixture)

    expect(trigger.textContent).toContain('florian')
    expect(trigger.getAttribute('aria-expanded')).toBe('true')
    const items = Array.from(
      fixture.nativeElement.querySelectorAll('[role="menu"] [gbtMenuItem]'),
    ) as HTMLElement[]
    expect(items.map((item) => item.textContent?.trim())).toEqual([
      'Mon compte',
      'Documentation',
      'Déconnexion',
    ])
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
    const trigger = openUserMenu(fixture)

    const accountLink: HTMLAnchorElement = fixture.nativeElement.querySelector(
      '[role="menu"] a[href="/account"]',
    )
    expect(accountLink).toBeTruthy()
    expect(accountLink.textContent).toContain('Mon compte')

    accountLink.click()
    fixture.detectChanges()
    expect(trigger.getAttribute('aria-expanded')).toBe('false')
  })

  describe('quick search', () => {
    function setup(me: { is_super_admin: boolean; is_organization_admin?: boolean }) {
      const fixture = TestBed.createComponent(AppShell)
      fixture.detectChanges()
      flushMe({ id: 'user-1', username: 'florian', ...me })
      fixture.detectChanges()
      return fixture
    }

    function openPalette(fixture: ReturnType<typeof setup>): void {
      fixture.nativeElement
        .querySelector('.app-shell__header gbt-command-palette-trigger button')
        .click()
      fixture.detectChanges()
      // Opening fetches fresh lists, to catch what another tab changed.
      for (const req of httpMock.match('/api/repositories')) {
        req.flush([])
      }
      for (const req of httpMock.match('/api/users')) {
        req.flush([])
      }
      fixture.detectChanges()
    }

    function type(fixture: ReturnType<typeof setup>, text: string): void {
      const field: HTMLInputElement = fixture.nativeElement.querySelector(
        '[role="dialog"] input[role="combobox"]',
      )
      field.value = text
      field.dispatchEvent(new Event('input'))
      fixture.detectChanges()
    }

    function groups(fixture: ReturnType<typeof setup>): Record<string, string[]> {
      const result: Record<string, string[]> = {}
      for (const group of Array.from<HTMLElement>(
        fixture.nativeElement.querySelectorAll('[role="group"]'),
      )) {
        const label = group.querySelector('.gbt-cp__group-label')!.textContent!.trim()
        result[label] = Array.from(group.querySelectorAll('.gbt-cp__item-label'), (item) =>
          item.textContent!.trim(),
        )
      }
      return result
    }

    const ADMINISTRATION = ['Tableau de bord', 'Utilisateurs', 'Organisations', 'Santé', 'Export']

    it('opens from a button in the header, the palette itself living outside it', () => {
      const fixture = setup({ is_super_admin: false })
      const trigger: HTMLButtonElement = fixture.nativeElement.querySelector(
        '.app-shell__header gbt-command-palette-trigger button',
      )
      expect(trigger.getAttribute('aria-label')).toBe('Rechercher ou aller à…')
      expect(trigger.getAttribute('aria-keyshortcuts')).toMatch(/^(Meta|Control)\+K$/)

      openPalette(fixture)

      // Outside the header, whose dark theme it would otherwise take on.
      expect(fixture.nativeElement.querySelector('app-quick-search [role="dialog"]')).not.toBeNull()
      expect(fixture.nativeElement.querySelector('.app-shell__header [role="dialog"]')).toBeNull()
    })

    it('offers a standard user no administration entry, whatever is typed', () => {
      const fixture = setup({ is_super_admin: false })
      openPalette(fixture)

      expect(groups(fixture)).toEqual({
        'Aller à': ['Explorer', 'Dépôts', 'Mon dépôt', 'Mon compte', 'Documentation'],
        Actions: ['Nouveau dépôt', 'Nouveau projet', 'Déconnexion'],
      })
      for (const query of [
        'admin',
        'tableau',
        'utilisateur',
        'organisation',
        'santé',
        'export',
        'réglages',
        'inviter',
      ]) {
        type(fixture, query)
        const shown = Object.values(groups(fixture)).flat()
        expect(
          shown.filter((label) => ADMINISTRATION.includes(label) || label === 'Réglages'),
        ).toEqual([])
        expect(shown).not.toContain('Inviter un utilisateur')
        expect(Object.keys(groups(fixture))).not.toContain('Utilisateurs')
      }
    })

    it("offers an organization admin its users and its organization's settings, not the instance's pages", () => {
      const fixture = setup({ is_super_admin: false, is_organization_admin: true })
      openPalette(fixture)

      expect(groups(fixture)['Aller à']).toEqual([
        'Explorer',
        'Dépôts',
        'Mon dépôt',
        'Utilisateurs',
        'Réglages',
        'Mon compte',
        'Documentation',
      ])
      expect(groups(fixture)['Actions']).not.toContain('Inviter un utilisateur')
    })

    it('offers a super-admin every administration page and the invitation', () => {
      const fixture = setup({ is_super_admin: true })
      openPalette(fixture)

      expect(groups(fixture)['Aller à']).toEqual([
        'Explorer',
        'Dépôts',
        'Mon dépôt',
        ...ADMINISTRATION,
        'Mon compte',
        'Documentation',
      ])
      expect(groups(fixture)['Actions']).toEqual([
        'Nouveau dépôt',
        'Nouveau projet',
        'Inviter un utilisateur',
        'Déconnexion',
      ])
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
    openUserMenu(fixture)

    const items: HTMLButtonElement[] = Array.from(
      fixture.nativeElement.querySelectorAll('[role="menu"] button[gbtMenuItem]'),
    )
    const logoutItem = items.find((el) => el.textContent?.includes('Déconnexion'))!
    logoutItem.click()

    expect(auth.token()).toBeNull()
    expect(navigate).toHaveBeenCalledWith('/login')
  })
})
