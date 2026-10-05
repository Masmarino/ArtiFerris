import { vi } from 'vitest'
import { Component } from '@angular/core'
import { TestBed } from '@angular/core/testing'
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing'
import { provideHttpClient } from '@angular/common/http'
import { Router, provideRouter } from '@angular/router'
import { MeService } from '../application/me.service'
import { meProviders } from '../infrastructure/me.providers'
import { readableCatalogProviders } from '../infrastructure/readable-catalog.providers'
import { NavItem } from '../nav-item'
import {
  proxiedEntry,
  readableDockerEntry,
  readableEntry,
  readableSearchResult,
} from '../testing/readable-catalog.fixtures'
import { authProviders } from '../../auth/infrastructure/auth.providers'
import { repositoryProviders } from '../../repositories/infrastructure/repository.providers'
import { RepositorySummary } from '../../repositories/domain/repository.entity'
import { userProviders } from '../../users/infrastructure/user.providers'
import { UserSummary } from '../../users/domain/user.entity'
import { QuickSearch } from './quick-search'

@Component({ template: '' })
class EmptyPage {}

const NAV: NavItem[] = [
  { action: 'explorer', icon: 'compass', text: 'Explorer', link: '/', exact: true },
  { action: 'repositories', icon: 'package', text: 'Dépôts', link: '/repositories' },
]

function repository(overrides: Partial<RepositorySummary>): RepositorySummary {
  return {
    id: 'r-1',
    name: 'npm-hosted',
    format: 'npm',
    repo_type: 'hosted',
    remote_url: null,
    remote_credentials_set: false,
    group_members: [],
    is_public: false,
    my_role: 'reader',
    owner_name: 'acme',
    owner_is_personal: false,
    ...overrides,
  } as RepositorySummary
}

function user(overrides: Partial<UserSummary>): UserSummary {
  return {
    id: 'u-2',
    username: 'camille',
    is_super_admin: false,
    organization_id: 'org-1',
    email: null,
    invitation_pending: false,
    created_at: '2026-01-01T00:00:00Z',
    ...overrides,
  } as UserSummary
}

const REPOSITORIES = [
  repository({ id: 'r-1', name: 'npm-hosted' }),
  repository({ id: 'r-2', name: 'docker-proxy', owner_name: 'florian', owner_is_personal: true }),
  repository({ id: 'r-3', name: 'maven-releases' }),
]

describe('QuickSearch', () => {
  let httpMock: HttpTestingController

  beforeEach(() => {
    TestBed.configureTestingModule({
      providers: [
        provideHttpClient(),
        provideHttpClientTesting(),
        ...authProviders,
        ...userProviders,
        ...repositoryProviders,
        ...meProviders,
        ...readableCatalogProviders,
        provideRouter([{ path: '**', component: EmptyPage }]),
      ],
    })
    httpMock = TestBed.inject(HttpTestingController)
  })

  afterEach(() => {
    // Missing under this Node; the service then keeps the list in memory, reset with TestBed.
    globalThis.localStorage?.clear()
    vi.useRealTimers()
  })

  function setup(options: { superAdmin?: boolean; organizationAdmin?: boolean } = {}) {
    const me = TestBed.inject(MeService)
    me.username.set('florian')
    me.isSuperAdmin.set(options.superAdmin ?? false)
    me.isOrganizationAdmin.set(options.organizationAdmin ?? false)
    const fixture = TestBed.createComponent(QuickSearch)
    fixture.componentRef.setInput('navItems', NAV)
    fixture.detectChanges()
    return fixture
  }

  type Fixture = ReturnType<typeof setup>

  function openPalette(
    fixture: Fixture,
    lists: { repositories?: RepositorySummary[]; users?: UserSummary[] } = {},
  ): void {
    fixture.componentInstance.show()
    fixture.detectChanges()
    for (const req of httpMock.match('/api/repositories')) {
      req.flush(lists.repositories ?? REPOSITORIES)
    }
    for (const req of httpMock.match('/api/users')) {
      req.flush(lists.users ?? [])
    }
    fixture.detectChanges()
  }

  function field(fixture: Fixture): HTMLInputElement {
    return fixture.nativeElement.querySelector('[role="dialog"] input[role="combobox"]')
  }

  function type(fixture: Fixture, text: string): void {
    field(fixture).value = text
    field(fixture).dispatchEvent(new Event('input'))
    fixture.detectChanges()
  }

  function press(fixture: Fixture, key: string): void {
    field(fixture).dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true }))
    fixture.detectChanges()
  }

  function groups(fixture: Fixture): Record<string, string[]> {
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

  const packageRequests = () => httpMock.match((req) => req.url === '/api/search')

  it('opens on Ctrl K and on "/" outside a field, and refreshes the lists on each opening', () => {
    const fixture = setup()

    document.dispatchEvent(
      new KeyboardEvent('keydown', { key: 'k', ctrlKey: true, bubbles: true, cancelable: true }),
    )
    fixture.detectChanges()
    expect(fixture.nativeElement.querySelector('[role="dialog"]')?.getAttribute('aria-label')).toBe(
      'Recherche rapide',
    )
    httpMock.expectOne('/api/repositories').flush([])

    press(fixture, 'Escape')
    document.dispatchEvent(
      new KeyboardEvent('keydown', { key: '/', bubbles: true, cancelable: true }),
    )
    fixture.detectChanges()
    expect(fixture.nativeElement.querySelector('[role="dialog"]')).not.toBeNull()
    httpMock.expectOne('/api/repositories').flush([])
  })

  it('lists the repositories once something is typed, filtered like the pages', () => {
    const fixture = setup()
    openPalette(fixture)
    expect(Object.keys(groups(fixture))).not.toContain('Dépôts')

    type(fixture, 'docker')

    expect(groups(fixture)['Dépôts']).toEqual(['docker-proxy'])
    const description = fixture.nativeElement.querySelector('.gbt-cp__item-description')
    expect(description.textContent.trim()).toBe('@florian')
  })

  it('lists users only to those who can open the users page', () => {
    const plain = setup()
    openPalette(plain)
    type(plain, 'cam')
    expect(Object.keys(groups(plain))).not.toContain('Utilisateurs')
    httpMock.expectNone('/api/users')
    plain.destroy()

    const admin = setup({ organizationAdmin: true })
    openPalette(admin, { users: [user({ username: 'camille' })] })
    type(admin, 'cam')
    expect(groups(admin)['Utilisateurs']).toEqual(['camille'])
  })

  it('remembers the repositories opened, per account, newest first, without the one shown', async () => {
    const fixture = setup()
    const router = TestBed.inject(Router)
    await router.navigateByUrl('/repositories/r-1')
    await router.navigateByUrl('/repositories/r-3/packages/npm/left-pad')
    await router.navigateByUrl('/repositories/r-2')
    await router.navigateByUrl('/repositories')

    openPalette(fixture)
    expect(groups(fixture)['Récents']).toEqual(['docker-proxy', 'maven-releases', 'npm-hosted'])

    await router.navigateByUrl('/repositories/r-2')
    fixture.detectChanges()
    expect(groups(fixture)['Récents']).toEqual(['maven-releases', 'npm-hosted'])

    TestBed.inject(MeService).username.set('camille')
    fixture.detectChanges()
    // camille has only just been here: the repository on screen is not offered back.
    expect(groups(fixture)['Récents']).toBeUndefined()
  })

  it('drops a recent repository that is no longer listed', async () => {
    const fixture = setup()
    await TestBed.inject(Router).navigateByUrl('/repositories/r-gone')
    await TestBed.inject(Router).navigateByUrl('/repositories')

    openPalette(fixture)

    expect(groups(fixture)['Récents']).toBeUndefined()
  })

  describe('packages', () => {
    beforeEach(() => vi.useFakeTimers())

    it.each(['a', ' a '])('does not ask the server below two characters: %j', (query) => {
      const fixture = setup()
      openPalette(fixture)
      type(fixture, query)
      vi.advanceTimersByTime(1000)

      expect(packageRequests()).toEqual([])
    })

    it('waits for a pause, sends the trimmed text once, and lists what comes back after the rest', () => {
      const fixture = setup()
      openPalette(fixture)

      type(fixture, ' le')
      type(fixture, ' lef ')
      vi.advanceTimersByTime(249)
      expect(packageRequests()).toEqual([])
      vi.advanceTimersByTime(1)
      const requests = packageRequests()
      expect(requests.map((req) => req.request.params.get('q'))).toEqual(['lef'])
      requests[0].flush(readableSearchResult([readableEntry(), proxiedEntry()]))
      fixture.detectChanges()

      expect(Object.keys(groups(fixture)).at(-1)).toBe('Paquets et images')
      expect(groups(fixture)['Paquets et images']).toEqual(['left-pad', 'lodash'])
      const descriptions = Array.from(
        fixture.nativeElement.querySelectorAll(
          '[role="group"]:last-child .gbt-cp__item-description',
        ) as NodeListOf<HTMLElement>,
        (el) => el.textContent!.trim(),
      )
      expect(descriptions[1]).toContain('npmjs-proxy (cache du proxy)')
    })

    it('never lets an older answer overwrite a newer one', () => {
      const fixture = setup()
      openPalette(fixture)

      type(fixture, 'left')
      vi.advanceTimersByTime(250)
      const [first] = packageRequests()
      type(fixture, 'api')
      vi.advanceTimersByTime(250)
      const [second] = packageRequests()
      expect(first.cancelled).toBe(true)
      second.flush(readableSearchResult([readableDockerEntry()]))
      fixture.detectChanges()

      expect(groups(fixture)['Paquets et images']).toEqual(['api'])
    })

    it('says the search failed when the server does and nothing else matches', () => {
      const fixture = setup()
      openPalette(fixture)

      type(fixture, 'zzz')
      vi.advanceTimersByTime(250)
      packageRequests()[0].flush('boom', { status: 500, statusText: 'Server Error' })
      fixture.detectChanges()

      expect(fixture.nativeElement.querySelector('.gbt-cp__empty').textContent.trim()).toBe(
        'La recherche a échoué',
      )
    })

    it('opens the package page of the one chosen', () => {
      const fixture = setup()
      const navigate = vi.spyOn(TestBed.inject(Router), 'navigate').mockResolvedValue(true)
      openPalette(fixture, { repositories: [] })

      type(fixture, 'api')
      vi.advanceTimersByTime(250)
      packageRequests()[0].flush(readableSearchResult([readableDockerEntry({ name: 'team/api' })]))
      fixture.detectChanges()
      press(fixture, 'Enter')

      expect(navigate).toHaveBeenCalledWith(
        ['/repositories', 'r-docker', 'packages', 'docker', 'team/api'],
        { queryParams: undefined },
      )
    })
  })

  it('opens the creation dialog of the page that holds it', () => {
    const fixture = setup()
    const navigate = vi.spyOn(TestBed.inject(Router), 'navigate').mockResolvedValue(true)
    openPalette(fixture)

    type(fixture, 'nouveau projet')
    press(fixture, 'Enter')

    expect(navigate).toHaveBeenCalledWith(['/my-repository'], { queryParams: { new: 'project' } })
  })

  it('signs out through the shell', () => {
    const fixture = setup()
    const logout = vi.fn()
    fixture.componentInstance.logout.subscribe(logout)
    openPalette(fixture)

    type(fixture, 'déconnexion')
    press(fixture, 'Enter')

    expect(logout).toHaveBeenCalledTimes(1)
  })
})
