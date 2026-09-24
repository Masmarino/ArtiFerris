import { TestBed } from '@angular/core/testing'
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing'
import { provideHttpClient } from '@angular/common/http'
import { SecurityLog } from './security-log'
import { userProviders } from '../../users/infrastructure/user.providers'
import { repositoryProviders } from '../../repositories/infrastructure/repository.providers'
import { adminProviders } from '../infrastructure/admin.providers'
import { organizationMembersProviders } from '../infrastructure/organization-members.providers'
import { ConfirmService } from '../../shared/confirm.service'
import { ToastService } from '../../shared/toast.service'

const page = (entries: unknown[], next_cursor: string | null = null) => ({ entries, next_cursor })

describe('SecurityLog', () => {
  afterEach(() => vi.useRealTimers())

  function render(
    entries: unknown[] = [],
    blocked: unknown[] = [],
    nextCursor: string | null = null,
    confirmed = true,
  ) {
    TestBed.configureTestingModule({
      providers: [
        provideHttpClient(),
        provideHttpClientTesting(),
        { provide: ConfirmService, useValue: { ask: vi.fn().mockResolvedValue(confirmed) } },
        ...userProviders,
        ...repositoryProviders,
        ...adminProviders,
        ...organizationMembersProviders,
      ],
    })
    const fixture = TestBed.createComponent(SecurityLog)
    const httpMock = TestBed.inject(HttpTestingController)
    fixture.detectChanges()
    httpMock.expectOne((r) => r.url === '/api/audit/events').flush(page(entries, nextCursor))
    httpMock
      .expectOne('/api/users')
      .flush([{ id: 'user-1', username: 'florian', is_super_admin: true }])
    httpMock.expectOne('/api/repositories').flush([
      {
        id: 'repo-1',
        name: 'my-repo',
        format: 'npm',
        repo_type: 'hosted',
        remote_url: null,
        group_members: [],
      },
    ])
    httpMock.expectOne('/api/admin/security/blocked').flush(blocked)
    fixture.detectChanges()
    return fixture
  }

  it('shows a loading state instead of an empty table while the request is in flight', () => {
    TestBed.configureTestingModule({
      providers: [
        provideHttpClient(),
        provideHttpClientTesting(),
        ...userProviders,
        ...repositoryProviders,
        ...adminProviders,
        ...organizationMembersProviders,
      ],
    })
    const fixture = TestBed.createComponent(SecurityLog)
    const httpMock = TestBed.inject(HttpTestingController)

    fixture.detectChanges()

    expect(fixture.nativeElement.textContent).toContain('Chargement…')

    httpMock.expectOne((r) => r.url === '/api/audit/events').flush(page([]))
    fixture.detectChanges()

    expect(fixture.nativeElement.textContent).not.toContain('Chargement…')

    // Drain the remaining requests this component fires on init so later
    // tests in this file don't inherit an unflushed backlog.
    httpMock.expectOne('/api/users').flush([])
    httpMock.expectOne('/api/repositories').flush([])
    httpMock.expectOne('/api/admin/security/blocked').flush([])
  })

  it('shows the IP address for a login failure, with the raw username from the payload', () => {
    const fixture = render([
      {
        aggregate_type: 'Security',
        aggregate_id: 'x',
        event_type: 'LoginFailed',
        payload: { username: 'attacker', ip: '203.0.113.7' },
        occurred_at: '2026-01-01T00:00:00Z',
        actor_id: null,
      },
    ])

    expect(fixture.componentInstance.rows()).toEqual([
      {
        occurred_at: '2026-01-01T00:00:00Z',
        event_type: 'Échec de connexion',
        actor: 'attacker',
        details: 'Depuis 203.0.113.7',
      },
    ])
  })

  it('resolves the actor_id to a username and shows the repository name and action for a denied access', () => {
    const fixture = render([
      {
        aggregate_type: 'Security',
        aggregate_id: 'x',
        event_type: 'AccessDenied',
        payload: { user_id: 'user-1', repository_id: 'repo-1', action: 'push' },
        occurred_at: '2026-01-01T00:00:00Z',
        actor_id: 'user-1',
      },
    ])

    expect(fixture.componentInstance.rows()).toEqual([
      {
        occurred_at: '2026-01-01T00:00:00Z',
        event_type: 'Accès refusé',
        actor: 'florian',
        details: 'Action « push » refusée sur my-repo',
      },
    ])
  })

  it('keeps rendering every row when an entry has a null payload or a null settings change', () => {
    const entry = (event_type: string, payload: unknown, actor_id: string | null = null) => ({
      aggregate_type: 'Security',
      aggregate_id: 'x',
      event_type,
      payload,
      occurred_at: '2026-01-01T00:00:00Z',
      actor_id,
    })
    const fixture = render([
      entry('LoginFailed', null),
      entry('AccessDenied', null, 'user-1'),
      entry('SystemSettingsChanged', {
        changes: [null, { setting: 'session_ttl_hours', before: 8, after: 12 }],
      }),
    ])

    const rows = fixture.componentInstance.rows()
    expect(rows).toHaveLength(3)
    expect(rows[0]).toMatchObject({ event_type: 'Échec de connexion', actor: '—', details: '' })
    expect(rows[1]).toMatchObject({ actor: 'florian', details: 'Action « ? » refusée sur ?' })
    expect(rows[2].details).toBe('Durée de session (h) : 8 → 12')
  })

  it('falls back to the raw actor_id when the user cannot be resolved', () => {
    const fixture = render([
      {
        aggregate_type: 'Security',
        aggregate_id: 'x',
        event_type: 'AccessDenied',
        payload: { user_id: 'ghost', repository_id: 'repo-1', action: 'pull' },
        occurred_at: '2026-01-01T00:00:00Z',
        actor_id: 'ghost',
      },
    ])

    expect(fixture.componentInstance.rows()[0].actor).toBe('ghost')
  })

  it('summarizes the event counts by type', () => {
    const fixture = render([
      {
        aggregate_type: 'Security',
        aggregate_id: 'a',
        event_type: 'LoginFailed',
        payload: { username: 'a', ip: '1.1.1.1' },
        occurred_at: '2026-01-01T00:00:00Z',
        actor_id: null,
      },
      {
        aggregate_type: 'Security',
        aggregate_id: 'b',
        event_type: 'LoginFailed',
        payload: { username: 'b', ip: '1.1.1.2' },
        occurred_at: '2026-01-01T00:01:00Z',
        actor_id: null,
      },
      {
        aggregate_type: 'Security',
        aggregate_id: 'c',
        event_type: 'AccessDenied',
        payload: { user_id: 'user-1', repository_id: 'repo-1', action: 'push' },
        occurred_at: '2026-01-01T00:02:00Z',
        actor_id: 'user-1',
      },
    ])

    expect(fixture.componentInstance.summary()).toEqual(
      expect.arrayContaining([
        { event_type: 'Échec de connexion', count: 2 },
        { event_type: 'Accès refusé', count: 1 },
      ]),
    )
  })

  it('renders the summary as a bar chart, most frequent event type first', () => {
    const fixture = render([
      {
        aggregate_type: 'Security',
        aggregate_id: 'a',
        event_type: 'LoginFailed',
        payload: { username: 'a', ip: '1.1.1.1' },
        occurred_at: '2026-01-01T00:00:00Z',
        actor_id: null,
      },
      {
        aggregate_type: 'Security',
        aggregate_id: 'b',
        event_type: 'LoginFailed',
        payload: { username: 'b', ip: '1.1.1.2' },
        occurred_at: '2026-01-01T00:01:00Z',
        actor_id: null,
      },
      {
        aggregate_type: 'Security',
        aggregate_id: 'c',
        event_type: 'AccessDenied',
        payload: { user_id: 'user-1', repository_id: 'repo-1', action: 'push' },
        occurred_at: '2026-01-01T00:02:00Z',
        actor_id: 'user-1',
      },
    ])

    const labels: (string | undefined)[] = Array.from(
      fixture.nativeElement.querySelectorAll('.gbt-dimension-card__label'),
    ).map((el: unknown) => (el as HTMLElement).childNodes[0]?.textContent?.trim())
    expect(labels).toEqual(['Échec de connexion', 'Accès refusé'])
  })

  it('does not render the summary section when there are no events', () => {
    const fixture = render()

    expect(fixture.nativeElement.textContent).not.toContain('Résumé')
  })

  it('shows currently blocked accounts with a human-readable remaining time', () => {
    const fixture = render([], [{ username: 'attacker', remaining_seconds: 125 }])

    const text = fixture.nativeElement.textContent as string
    expect(text).toContain('attacker')
    expect(text).toContain('3 minutes')
  })

  describe('unlocking a blocked account', () => {
    const shared = { username: 'login-user:alice', remaining_seconds: 300 }

    const unlockButtons = (fixture: { nativeElement: HTMLElement }) =>
      Array.from(fixture.nativeElement.querySelectorAll('li button')).filter(
        (button) => button.textContent?.trim() === 'Débloquer',
      ) as HTMLButtonElement[]

    async function clickUnlock(fixture: ReturnType<typeof render>) {
      unlockButtons(fixture)[0].click()
      await fixture.whenStable()
    }

    it('lists a login key by its username, with the organization ones told apart', () => {
      const fixture = render(
        [],
        [
          shared,
          {
            username: 'login-org-user:0b8f6a58-1c3e-4a7b-9d2f-5e6a7b8c9d0e:bob',
            remaining_seconds: 60,
          },
        ],
      )

      const text = fixture.nativeElement.textContent as string
      expect(text).toContain('alice — débloqué dans 5 minutes')
      expect(text).toContain('bob (organisation)')
      expect(text).not.toContain('login-user:')
    })

    it('offers no unlock button for keys that are not tied to a username', () => {
      const fixture = render(
        [],
        [
          { username: 'mfa:3f2b1c00-0000-4000-8000-000000000001', remaining_seconds: 60 },
          { username: 'login-ip:203.0.113.0', remaining_seconds: 60 },
        ],
      )

      expect(unlockButtons(fixture)).toHaveLength(0)
      expect(fixture.nativeElement.textContent).toContain('mfa:3f2b1c00')
    })

    it('unlocks by the bare username after confirmation, then refreshes the list', async () => {
      const fixture = render([], [shared])
      const httpMock = TestBed.inject(HttpTestingController)

      await clickUnlock(fixture)

      expect(TestBed.inject(ConfirmService).ask).toHaveBeenCalledWith(
        expect.objectContaining({ heading: 'Débloquer le compte' }),
      )
      const req = httpMock.expectOne('/api/admin/login-throttle/alice')
      expect(req.request.method).toBe('DELETE')
      req.flush(null, { status: 204, statusText: 'No Content' })
      httpMock.expectOne('/api/admin/security/blocked').flush([])
      fixture.detectChanges()

      expect(TestBed.inject(ToastService).toasts().at(-1)).toMatchObject({
        variant: 'success',
        message: 'alice a été débloqué·e.',
      })
      expect(fixture.nativeElement.textContent).not.toContain('Comptes actuellement bloqués')
    })

    it('sends nothing when the confirmation is cancelled', async () => {
      const fixture = render([], [shared], null, false)
      const httpMock = TestBed.inject(HttpTestingController)

      await clickUnlock(fixture)

      httpMock.expectNone('/api/admin/login-throttle/alice')
    })

    it.each([
      [403, "Vous n'avez pas le droit de débloquer alice."],
      [404, 'alice est introuvable : rien à débloquer.'],
      [500, 'Échec du déblocage de alice.'],
    ])('explains a %i in French and keeps the row', async (status, message) => {
      const fixture = render([], [shared])
      const httpMock = TestBed.inject(HttpTestingController)

      await clickUnlock(fixture)
      httpMock
        .expectOne('/api/admin/login-throttle/alice')
        .flush({ error: 'x' }, { status, statusText: 'x' })
      fixture.detectChanges()

      expect(TestBed.inject(ToastService).toasts().at(-1)).toMatchObject({
        variant: 'error',
        message,
      })
      expect(fixture.nativeElement.textContent).toContain('alice — débloqué dans')
      httpMock.expectNone('/api/admin/security/blocked')
    })
  })

  it('shows nothing under "Comptes actuellement bloqués" when no account is blocked', () => {
    const fixture = render([], [])

    expect(fixture.nativeElement.textContent).not.toContain('Comptes actuellement bloqués')
  })

  it('formats remaining time under a minute distinctly', () => {
    TestBed.configureTestingModule({
      providers: [
        provideHttpClient(),
        provideHttpClientTesting(),
        ...userProviders,
        ...repositoryProviders,
        ...adminProviders,
        ...organizationMembersProviders,
      ],
    })
    const fixture = TestBed.createComponent(SecurityLog)
    expect(fixture.componentInstance.formatRemainingTime(30)).toBe("moins d'une minute")
    expect(fixture.componentInstance.formatRemainingTime(90)).toBe('2 minutes')
  })

  it('downloads a CSV of the currently loaded rows', () => {
    const createObjectURL = vi.spyOn(URL, 'createObjectURL').mockReturnValue('blob:mock')
    const revokeObjectURL = vi.spyOn(URL, 'revokeObjectURL').mockImplementation(() => undefined)
    vi.useFakeTimers()
    const fixture = render([
      {
        aggregate_type: 'Security',
        aggregate_id: 'x',
        event_type: 'LoginFailed',
        payload: { username: 'attacker', ip: '203.0.113.7' },
        occurred_at: '2026-01-01T00:00:00Z',
        actor_id: null,
      },
    ])

    fixture.componentInstance.downloadCsv()

    expect(createObjectURL).toHaveBeenCalled()
    const blob = createObjectURL.mock.calls[0][0] as Blob
    expect(blob.type).toContain('text/csv')
    // Revoked a moment later: some browsers only start reading the blob after click() returns.
    expect(revokeObjectURL).not.toHaveBeenCalled()
    vi.advanceTimersByTime(1000)
    expect(revokeObjectURL).toHaveBeenCalledWith('blob:mock')
  })

  it('disables the CSV download button when there are no rows', () => {
    const fixture = render()

    const button: HTMLButtonElement = fixture.nativeElement.querySelector('gbt-button button')
    expect(button.disabled).toBe(true)
  })

  describe('scoped to an organization', () => {
    function renderScoped(entries: unknown[] = []) {
      TestBed.configureTestingModule({
        providers: [
          provideHttpClient(),
          provideHttpClientTesting(),
          ...userProviders,
          ...repositoryProviders,
          ...adminProviders,
          ...organizationMembersProviders,
        ],
      })
      const fixture = TestBed.createComponent(SecurityLog)
      fixture.componentRef.setInput('organizationId', 'org-1')
      const httpMock = TestBed.inject(HttpTestingController)
      fixture.detectChanges()
      httpMock.expectOne((r) => r.url === '/api/audit/events').flush(page(entries))
      httpMock.expectOne('/api/organizations/org-1/users').flush([
        {
          id: 'user-1',
          username: 'florian',
          email: null,
          is_organization_admin: true,
          invitation_pending: false,
        },
      ])
      httpMock.expectOne('/api/repositories').flush([])
      fixture.detectChanges()
      return { fixture, httpMock }
    }

    it('resolves the actor via the organization members endpoint, not the global user list', () => {
      const { fixture } = renderScoped([
        {
          aggregate_type: 'Security',
          aggregate_id: 'x',
          event_type: 'AccessDenied',
          payload: { user_id: 'user-1', repository_id: 'repo-1', action: 'push' },
          occurred_at: '2026-01-01T00:00:00Z',
          actor_id: 'user-1',
        },
      ])

      expect(fixture.componentInstance.rows()[0].actor).toBe('florian')
    })

    it('does not call the blocked-accounts endpoint or render the blocked-accounts panel', () => {
      const { fixture, httpMock } = renderScoped()

      httpMock.expectNone('/api/admin/security/blocked')
      expect(fixture.nativeElement.textContent).not.toContain('Comptes actuellement bloqués')
    })
  })

  it('does not apply a stale org-1 response after switching to org-2 before org-1 resolves', () => {
    TestBed.configureTestingModule({
      providers: [
        provideHttpClient(),
        provideHttpClientTesting(),
        ...userProviders,
        ...repositoryProviders,
        ...adminProviders,
        ...organizationMembersProviders,
      ],
    })
    const fixture = TestBed.createComponent(SecurityLog)
    const httpMock = TestBed.inject(HttpTestingController)

    fixture.componentRef.setInput('organizationId', 'org-1')
    fixture.detectChanges()
    const org1Req = httpMock.expectOne(
      (r) => r.url === '/api/audit/events' && r.params.get('organization_id') === 'org-1',
    )
    httpMock.expectOne('/api/organizations/org-1/users').flush([])
    httpMock.expectOne('/api/repositories').flush([])

    // Switch away from org-1 before its request resolves.
    fixture.componentRef.setInput('organizationId', 'org-2')
    fixture.detectChanges()
    const org2Req = httpMock.expectOne(
      (r) => r.url === '/api/audit/events' && r.params.get('organization_id') === 'org-2',
    )
    httpMock.expectOne('/api/organizations/org-2/users').flush([])
    // repositoriesService.list() caches its result via shareReplay — the first flush above
    // already satisfied this second effect run too, so no second request is issued here.

    // Resolve the current org-2 request first, then the stale org-1 request second —
    // this is the racy order the guard exists to handle: a slow first request that
    // finally resolves after a faster second request has already applied its result.
    org2Req.flush(
      page([
        {
          aggregate_type: 'Security',
          aggregate_id: 'org2-event',
          event_type: 'LoginFailed',
          payload: { username: 'org2user', ip: '2.2.2.2' },
          occurred_at: '2026-01-02T00:00:00Z',
          actor_id: null,
        },
      ]),
    )
    org1Req.flush(
      page([
        {
          aggregate_type: 'Security',
          aggregate_id: 'org1-event',
          event_type: 'LoginFailed',
          payload: { username: 'org1user', ip: '1.1.1.1' },
          occurred_at: '2026-01-01T00:00:00Z',
          actor_id: null,
        },
      ]),
    )
    fixture.detectChanges()

    expect(fixture.componentInstance.rows().length).toBe(1)
    expect(fixture.componentInstance.rows()[0].actor).toBe('org2user')
    expect(fixture.nativeElement.textContent).not.toContain('org1user')
  })

  it('shows an error state instead of an infinite loading spinner when the audit query fails', () => {
    TestBed.configureTestingModule({
      providers: [
        provideHttpClient(),
        provideHttpClientTesting(),
        ...userProviders,
        ...repositoryProviders,
        ...adminProviders,
        ...organizationMembersProviders,
      ],
    })
    const fixture = TestBed.createComponent(SecurityLog)
    const httpMock = TestBed.inject(HttpTestingController)

    fixture.detectChanges()
    httpMock
      .expectOne((r) => r.url === '/api/audit/events')
      .flush('network error', { status: 500, statusText: 'Internal Server Error' })
    fixture.detectChanges()

    expect(fixture.componentInstance.loading()).toBe(false)
    expect(fixture.componentInstance.loadFailed()).toBe(true)
    expect(fixture.nativeElement.textContent).not.toContain('Chargement…')

    // Drain the other in-flight requests fired by this same effect run so this
    // test doesn't leave a backlog for the next one.
    httpMock.expectOne('/api/users').flush([])
    httpMock.expectOne('/api/repositories').flush([])
    httpMock.expectOne('/api/admin/security/blocked').flush([])
  })

  describe('pagination', () => {
    const event = (id: string) => ({
      aggregate_type: 'Security',
      aggregate_id: id,
      event_type: 'LoginFailed',
      payload: { username: `user-${id}`, ip: '1.1.1.1' },
      occurred_at: `2026-01-01T00:00:0${id}Z`,
      actor_id: null,
    })

    function renderPaged(first: ReturnType<typeof page>) {
      TestBed.configureTestingModule({
        providers: [
          provideHttpClient(),
          provideHttpClientTesting(),
          ...userProviders,
          ...repositoryProviders,
          ...adminProviders,
          ...organizationMembersProviders,
        ],
      })
      const fixture = TestBed.createComponent(SecurityLog)
      const httpMock = TestBed.inject(HttpTestingController)
      fixture.detectChanges()
      httpMock.expectOne((r) => r.url === '/api/audit/events').flush(first)
      httpMock
        .expectOne('/api/users')
        .flush([{ id: 'user-1', username: 'florian', is_super_admin: true }])
      httpMock.expectOne('/api/repositories').flush([])
      httpMock.expectOne('/api/admin/security/blocked').flush([])
      fixture.detectChanges()
      return { fixture, httpMock }
    }

    it('appends the next page of the instance-level log, cursor and Security filter included', () => {
      const { fixture, httpMock } = renderPaged(page([event('1')], 'c1'))

      const button = Array.from(
        (fixture.nativeElement as HTMLElement).querySelectorAll('button'),
      ).find((b) => b.textContent?.includes('Charger plus'))
      expect(button).toBeTruthy()
      button?.click()

      const next = httpMock.expectOne((r) => r.params.has('cursor'))
      expect(next.request.params.get('cursor')).toBe('c1')
      expect(next.request.params.get('aggregate_type')).toBe('Security')
      expect(next.request.params.has('organization_id')).toBe(false)
      next.flush(page([event('2')]))
      fixture.detectChanges()

      expect(fixture.componentInstance.rows()).toHaveLength(2)
      expect(fixture.nativeElement.textContent).not.toContain('Charger plus')
    })

    it('keeps the loaded rows when the next page fails', () => {
      const { fixture, httpMock } = renderPaged(page([event('1')], 'c1'))

      fixture.componentInstance.loadMore()
      httpMock
        .expectOne((r) => r.params.has('cursor'))
        .flush('boom', { status: 500, statusText: 'Internal Server Error' })
      fixture.detectChanges()

      expect(fixture.componentInstance.rows()).toHaveLength(1)
      expect(fixture.nativeElement.textContent).toContain('Impossible de charger la suite')
    })

    it('names a successful login and its method in French', () => {
      const { fixture } = renderPaged(
        page([
          {
            aggregate_type: 'Security',
            aggregate_id: 's1',
            event_type: 'LoginSucceeded',
            payload: { method: 'password', second_factor: 'totp', user_id: 'user-1' },
            occurred_at: '2026-01-01T00:00:00Z',
            actor_id: 'user-1',
          },
        ]),
      )

      const text = fixture.nativeElement.textContent as string
      expect(text).toContain('Connexion réussie')
      expect(text).toContain('Via mot de passe + application TOTP')
      expect(text).toContain('florian')
    })
  })

  describe('CSV export of a partly loaded log', () => {
    // Earlier tests in the file leave their URL spies behind.
    beforeEach(() => vi.restoreAllMocks())
    afterEach(() => vi.restoreAllMocks())

    const failure = (n: number) => ({
      aggregate_type: 'Security',
      aggregate_id: `agg-${n}`,
      event_type: 'LoginFailed',
      payload: { username: `user-${n}`, ip: '203.0.113.7' },
      occurred_at: '2026-01-01T00:00:00Z',
      actor_id: null,
    })
    const range = (from: number, count: number) =>
      Array.from({ length: count }, (_, i) => failure(from + i))

    it('fetches the missing pages and exports every event with its French label', async () => {
      const createObjectURL = vi.spyOn(URL, 'createObjectURL').mockReturnValue('blob:mock')
      vi.spyOn(URL, 'revokeObjectURL').mockImplementation(() => undefined)
      const fixture = render(range(0, 2), [], 'c1')
      const httpMock = TestBed.inject(HttpTestingController)

      fixture.componentInstance.downloadCsv()
      const request = httpMock.expectOne((r) => r.params.get('cursor') === 'c1')
      expect(request.request.params.get('aggregate_type')).toBe('Security')
      request.flush(page(range(2, 3)))

      const csv = await (createObjectURL.mock.calls[0][0] as Blob).text()
      expect(csv.split('\r\n')).toHaveLength(1 + 5)
      expect(csv).toContain('user-4')
      expect(csv).toContain('Échec de connexion')
      expect(fixture.componentInstance.exportPartial()).toBe(false)
    })

    it('produces no file and says so when a page fails midway', () => {
      const createObjectURL = vi.spyOn(URL, 'createObjectURL').mockReturnValue('blob:mock')
      const fixture = render(range(0, 2), [], 'c1')
      const httpMock = TestBed.inject(HttpTestingController)

      fixture.componentInstance.downloadCsv()
      httpMock
        .expectOne((r) => r.params.get('cursor') === 'c1')
        .flush({}, { status: 500, statusText: 'Error' })
      fixture.detectChanges()

      expect(createObjectURL).not.toHaveBeenCalled()
      expect(fixture.nativeElement.querySelector('[role="alert"]')?.textContent).toContain(
        "L'export a échoué",
      )
    })

    it('can be cancelled, leaving no file behind', () => {
      const createObjectURL = vi.spyOn(URL, 'createObjectURL').mockReturnValue('blob:mock')
      const fixture = render(range(0, 2), [], 'c1')
      const httpMock = TestBed.inject(HttpTestingController)

      fixture.componentInstance.downloadCsv()
      fixture.detectChanges()
      expect(fixture.nativeElement.textContent).toContain('Export en cours… 2 événements récupérés')
      const pending = httpMock.expectOne((r) => r.params.get('cursor') === 'c1')
      fixture.componentInstance.cancelExport()

      expect(pending.cancelled).toBe(true)
      expect(createObjectURL).not.toHaveBeenCalled()
    })

    it('cuts the file at the row bound and labels it partial', async () => {
      const createObjectURL = vi.spyOn(URL, 'createObjectURL').mockReturnValue('blob:mock')
      vi.spyOn(URL, 'revokeObjectURL').mockImplementation(() => undefined)
      const fixture = render(range(0, 100), [], 'c1')
      const httpMock = TestBed.inject(HttpTestingController)

      fixture.componentInstance.downloadCsv()
      httpMock.expectOne((r) => r.params.get('cursor') === 'c1').flush(page(range(100, 5000), 'c2'))
      httpMock
        .expectOne((r) => r.params.get('cursor') === 'c2')
        .flush(page(range(5100, 5000), 'c3'))
      fixture.detectChanges()

      const csv = await (createObjectURL.mock.calls[0][0] as Blob).text()
      expect(csv.split('\r\n')).toHaveLength(1 + 10_000)
      expect(fixture.nativeElement.textContent).toContain('10000 événements les plus récents')
    })

    it('says what the summary and the export cover while more events exist', () => {
      const fixture = render(range(0, 2), [], 'c1')

      const text = fixture.nativeElement.textContent as string
      expect(text).toContain('Calculé sur les 2 événements chargés')
      expect(text).toContain("l'export récupère aussi les suivants")
    })
  })
})
