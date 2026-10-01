import { TestBed } from '@angular/core/testing'
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing'
import { provideHttpClient } from '@angular/common/http'
import { AuditLog } from './audit-log'
import { adminProviders } from '../infrastructure/admin.providers'

const page = (entries: unknown[], next_cursor: string | null = null) => ({ entries, next_cursor })

describe('AuditLog', () => {
  afterEach(() => vi.useRealTimers())

  it('shows a loading state instead of an empty table while the request is in flight', () => {
    TestBed.configureTestingModule({
      providers: [provideHttpClient(), provideHttpClientTesting(), ...adminProviders],
    })
    const fixture = TestBed.createComponent(AuditLog)
    const httpMock = TestBed.inject(HttpTestingController)

    fixture.detectChanges()

    expect(fixture.nativeElement.textContent).toContain('Chargement…')

    httpMock.expectOne((r) => r.url === '/api/audit/events').flush(page([]))
    fixture.detectChanges()

    expect(fixture.nativeElement.textContent).not.toContain('Chargement…')
  })

  it('excludes Security events server-side rather than filtering them client-side', () => {
    TestBed.configureTestingModule({
      providers: [provideHttpClient(), provideHttpClientTesting(), ...adminProviders],
    })
    const fixture = TestBed.createComponent(AuditLog)
    const httpMock = TestBed.inject(HttpTestingController)

    fixture.detectChanges()

    const req = httpMock.expectOne((r) => r.url === '/api/audit/events')
    expect(req.request.method).toBe('GET')
    expect(req.request.params.get('exclude_aggregate_type')).toBe('Security')
    // The server filters: the client must not drop rows itself.
    expect(req.request.urlWithParams).toContain('exclude_aggregate_type=Security')

    req.flush(
      page([
        {
          aggregate_type: 'Permission',
          aggregate_id: 'repo-1',
          event_type: 'PermissionGranted',
          payload: {},
          occurred_at: '2026-01-01T00:00:00Z',
          actor_id: 'user-1',
        },
      ]),
    )

    expect(fixture.componentInstance.entries().length).toBe(1)
    expect(fixture.componentInstance.entries()[0].aggregate_type).toBe('Permission')
    httpMock.verify()
  })

  it('does not filter the response client-side, even if a Security event were present', () => {
    TestBed.configureTestingModule({
      providers: [provideHttpClient(), provideHttpClientTesting(), ...adminProviders],
    })
    const fixture = TestBed.createComponent(AuditLog)
    const httpMock = TestBed.inject(HttpTestingController)

    fixture.detectChanges()

    const req = httpMock.expectOne((r) => r.url === '/api/audit/events')
    req.flush(
      page([
        {
          aggregate_type: 'Security',
          aggregate_id: 'security-1',
          event_type: 'LoginFailed',
          payload: {},
          occurred_at: '2026-01-01T00:00:01Z',
          actor_id: null,
        },
        {
          aggregate_type: 'Permission',
          aggregate_id: 'repo-1',
          event_type: 'PermissionGranted',
          payload: {},
          occurred_at: '2026-01-01T00:00:00Z',
          actor_id: 'user-1',
        },
      ]),
    )

    expect(fixture.componentInstance.entries().length).toBe(2)
    expect(fixture.componentInstance.entries().map((e) => e.aggregate_type)).toEqual([
      'Security',
      'Permission',
    ])
    httpMock.verify()
  })

  it('summarizes entries by aggregate type as a bar chart, most frequent first', () => {
    TestBed.configureTestingModule({
      providers: [provideHttpClient(), provideHttpClientTesting(), ...adminProviders],
    })
    const fixture = TestBed.createComponent(AuditLog)
    const httpMock = TestBed.inject(HttpTestingController)

    fixture.detectChanges()

    httpMock
      .expectOne((r) => r.url === '/api/audit/events')
      .flush(
        page([
          {
            aggregate_type: 'Permission',
            aggregate_id: 'a',
            event_type: 'PermissionGranted',
            payload: {},
            occurred_at: '2026-01-01T00:00:00Z',
            actor_id: 'user-1',
          },
          {
            aggregate_type: 'Permission',
            aggregate_id: 'b',
            event_type: 'PermissionRevoked',
            payload: {},
            occurred_at: '2026-01-01T00:01:00Z',
            actor_id: 'user-1',
          },
          {
            aggregate_type: 'PackageRepository',
            aggregate_id: 'c',
            event_type: 'RepositoryCreated',
            payload: {},
            occurred_at: '2026-01-01T00:02:00Z',
            actor_id: 'user-1',
          },
        ]),
      )
    fixture.detectChanges()

    const labels: (string | undefined)[] = Array.from(
      fixture.nativeElement.querySelectorAll('.gbt-dimension-card__label'),
    ).map((el: unknown) => (el as HTMLElement).childNodes[0]?.textContent?.trim())
    expect(labels).toEqual(['Permission', 'PackageRepository'])
  })

  it('does not render the summary section when there are no entries', () => {
    TestBed.configureTestingModule({
      providers: [provideHttpClient(), provideHttpClientTesting(), ...adminProviders],
    })
    const fixture = TestBed.createComponent(AuditLog)
    const httpMock = TestBed.inject(HttpTestingController)

    fixture.detectChanges()
    httpMock.expectOne((r) => r.url === '/api/audit/events').flush(page([]))
    fixture.detectChanges()

    expect(fixture.nativeElement.textContent).not.toContain('Résumé')
  })

  it('downloads a CSV of the currently loaded entries', () => {
    TestBed.configureTestingModule({
      providers: [provideHttpClient(), provideHttpClientTesting(), ...adminProviders],
    })
    const fixture = TestBed.createComponent(AuditLog)
    const httpMock = TestBed.inject(HttpTestingController)
    const createObjectURL = vi.spyOn(URL, 'createObjectURL').mockReturnValue('blob:mock')
    const revokeObjectURL = vi.spyOn(URL, 'revokeObjectURL').mockImplementation(() => undefined)
    vi.useFakeTimers()

    fixture.detectChanges()
    httpMock
      .expectOne((r) => r.url === '/api/audit/events')
      .flush(
        page([
          {
            aggregate_type: 'Permission',
            aggregate_id: 'repo-1',
            event_type: 'PermissionGranted',
            payload: { role: 'write' },
            occurred_at: '2026-01-01T00:00:00Z',
            actor_id: 'user-1',
          },
        ]),
      )

    fixture.componentInstance.downloadCsv()

    expect(createObjectURL).toHaveBeenCalled()
    const blob = createObjectURL.mock.calls[0][0] as Blob
    expect(blob.type).toContain('text/csv')
    // Revoked a moment later: some browsers read the blob after click() returns.
    expect(revokeObjectURL).not.toHaveBeenCalled()
    vi.advanceTimersByTime(1000)
    expect(revokeObjectURL).toHaveBeenCalledWith('blob:mock')
  })

  it('disables the CSV download button when there are no entries', () => {
    TestBed.configureTestingModule({
      providers: [provideHttpClient(), provideHttpClientTesting(), ...adminProviders],
    })
    const fixture = TestBed.createComponent(AuditLog)
    const httpMock = TestBed.inject(HttpTestingController)

    fixture.detectChanges()
    httpMock.expectOne((r) => r.url === '/api/audit/events').flush(page([]))
    fixture.detectChanges()

    const button: HTMLButtonElement = fixture.nativeElement.querySelector('gbt-button button')
    expect(button.disabled).toBe(true)
  })

  it('re-fetches when organizationId changes to a different organization — the component is reused, not recreated, across a super-admin switching organizations', () => {
    TestBed.configureTestingModule({
      providers: [provideHttpClient(), provideHttpClientTesting(), ...adminProviders],
    })
    const fixture = TestBed.createComponent(AuditLog)
    const httpMock = TestBed.inject(HttpTestingController)

    fixture.detectChanges()
    httpMock
      .expectOne((r) => r.url === '/api/audit/events' && !r.params.has('organization_id'))
      .flush(page([]))

    fixture.componentRef.setInput('organizationId', 'org-1')
    fixture.detectChanges()
    httpMock
      .expectOne((r) => r.params.get('organization_id') === 'org-1')
      .flush(
        page([
          {
            aggregate_type: 'Repository',
            aggregate_id: 'r1',
            event_type: 'Created',
            payload: {},
            occurred_at: '2026-01-01T00:00:00Z',
            actor_id: null,
          },
        ]),
      )
    fixture.detectChanges()
    expect(fixture.nativeElement.textContent).toContain('Repository')

    fixture.componentRef.setInput('organizationId', 'org-2')
    fixture.detectChanges()
    httpMock.expectOne((r) => r.params.get('organization_id') === 'org-2').flush(page([]))
    fixture.detectChanges()

    expect(fixture.nativeElement.textContent).not.toContain('Repository')
  })

  it('empties the entries and the cursor as soon as the organization changes', () => {
    TestBed.configureTestingModule({
      providers: [provideHttpClient(), provideHttpClientTesting(), ...adminProviders],
    })
    const fixture = TestBed.createComponent(AuditLog)
    const httpMock = TestBed.inject(HttpTestingController)
    fixture.componentRef.setInput('organizationId', 'org-1')
    fixture.detectChanges()
    httpMock
      .expectOne((r) => r.params.get('organization_id') === 'org-1')
      .flush(
        page(
          [
            {
              aggregate_type: 'Repository',
              aggregate_id: 'r1',
              event_type: 'Created',
              payload: {},
              occurred_at: '2026-01-01T00:00:00Z',
              actor_id: null,
            },
          ],
          'cursor-1',
        ),
      )

    fixture.componentRef.setInput('organizationId', 'org-2')
    fixture.detectChanges()

    expect(fixture.componentInstance.entries()).toEqual([])
    expect(fixture.componentInstance.nextCursor()).toBeNull()
    httpMock.expectOne((r) => r.params.get('organization_id') === 'org-2').flush(page([]))
  })

  it('does not apply a stale org-1 response after switching to org-2 before org-1 resolves', () => {
    TestBed.configureTestingModule({
      providers: [provideHttpClient(), provideHttpClientTesting(), ...adminProviders],
    })
    const fixture = TestBed.createComponent(AuditLog)
    const httpMock = TestBed.inject(HttpTestingController)

    fixture.detectChanges()
    httpMock
      .expectOne((r) => r.url === '/api/audit/events' && !r.params.has('organization_id'))
      .flush(page([]))

    fixture.componentRef.setInput('organizationId', 'org-1')
    fixture.detectChanges()
    const org1Req = httpMock.expectOne((r) => r.params.get('organization_id') === 'org-1')

    // Switch away from org-1 before its request resolves.
    fixture.componentRef.setInput('organizationId', 'org-2')
    fixture.detectChanges()
    const org2Req = httpMock.expectOne((r) => r.params.get('organization_id') === 'org-2')

    // Resolve org-2 first, then the stale org-1: the racy order the guard handles.
    org2Req.flush(
      page([
        {
          aggregate_type: 'Permission',
          aggregate_id: 'org2-repo',
          event_type: 'PermissionGranted',
          payload: {},
          occurred_at: '2026-01-02T00:00:00Z',
          actor_id: null,
        },
      ]),
    )
    org1Req.flush(
      page([
        {
          aggregate_type: 'Repository',
          aggregate_id: 'org1-repo',
          event_type: 'Created',
          payload: {},
          occurred_at: '2026-01-01T00:00:00Z',
          actor_id: null,
        },
      ]),
    )
    fixture.detectChanges()

    expect(fixture.componentInstance.entries().length).toBe(1)
    expect(fixture.componentInstance.entries()[0].aggregate_id).toBe('org2-repo')
    expect(fixture.nativeElement.textContent).not.toContain('org1-repo')
  })

  it('shows an error state instead of an infinite loading spinner when the audit query fails', () => {
    TestBed.configureTestingModule({
      providers: [provideHttpClient(), provideHttpClientTesting(), ...adminProviders],
    })
    const fixture = TestBed.createComponent(AuditLog)
    const httpMock = TestBed.inject(HttpTestingController)

    fixture.detectChanges()
    httpMock
      .expectOne((r) => r.url === '/api/audit/events')
      .flush('network error', { status: 500, statusText: 'Internal Server Error' })
    fixture.detectChanges()

    expect(fixture.componentInstance.loading()).toBe(false)
    expect(fixture.componentInstance.loadFailed()).toBe(true)
    expect(fixture.nativeElement.textContent).not.toContain('Chargement…')
  })

  describe('pagination', () => {
    const entry = (id: string, eventType = 'PermissionGranted') => ({
      aggregate_type: 'Permission',
      aggregate_id: id,
      event_type: eventType,
      payload: {},
      occurred_at: `2026-01-01T00:00:0${id}Z`,
      actor_id: null,
    })

    function render() {
      TestBed.configureTestingModule({
        providers: [provideHttpClient(), provideHttpClientTesting(), ...adminProviders],
      })
      const fixture = TestBed.createComponent(AuditLog)
      const httpMock = TestBed.inject(HttpTestingController)
      fixture.detectChanges()
      return { fixture, httpMock }
    }

    const loadMoreButton = (root: HTMLElement) =>
      Array.from(root.querySelectorAll('button')).find((b) =>
        /Charger plus|Chargement…/.test(b.textContent ?? ''),
      )

    it('offers "Charger plus" only while the server reports a next page', () => {
      const { fixture, httpMock } = render()
      httpMock.expectOne((r) => r.url === '/api/audit/events').flush(page([entry('1')], 'c1'))
      fixture.detectChanges()

      expect(loadMoreButton(fixture.nativeElement)).toBeTruthy()

      fixture.componentInstance.loadMore()
      httpMock.expectOne((r) => r.params.get('cursor') === 'c1').flush(page([entry('2')], null))
      fixture.detectChanges()

      expect(loadMoreButton(fixture.nativeElement)).toBeUndefined()
    })

    it('appends the next page, sending the cursor along with the same filters', () => {
      const { fixture, httpMock } = render()
      fixture.componentRef.setInput('organizationId', 'org-1')
      fixture.detectChanges()
      httpMock
        .expectOne((r) => r.params.get('organization_id') === 'org-1' && !r.params.has('cursor'))
        .flush(page([entry('1')], 'c1'))
      httpMock.match((r) => !r.params.has('organization_id')).forEach((r) => r.flush(page([])))

      fixture.componentInstance.loadMore()

      const next = httpMock.expectOne((r) => r.params.has('cursor'))
      expect(next.request.params.get('cursor')).toBe('c1')
      expect(next.request.params.get('organization_id')).toBe('org-1')
      expect(next.request.params.get('exclude_aggregate_type')).toBe('Security')
      next.flush(page([entry('2')], null))

      expect(fixture.componentInstance.entries().map((e) => e.aggregate_id)).toEqual(['1', '2'])
      expect(fixture.componentInstance.nextCursor()).toBeNull()
    })

    it('does not fire a second request while one page is loading', () => {
      const { fixture, httpMock } = render()
      httpMock.expectOne((r) => r.url === '/api/audit/events').flush(page([entry('1')], 'c1'))

      fixture.componentInstance.loadMore()
      fixture.componentInstance.loadMore()

      expect(httpMock.match((r) => r.params.has('cursor'))).toHaveLength(1)
    })

    it('keeps what is loaded and offers a retry when the next page fails', () => {
      const { fixture, httpMock } = render()
      httpMock.expectOne((r) => r.url === '/api/audit/events').flush(page([entry('1')], 'c1'))

      fixture.componentInstance.loadMore()
      httpMock
        .expectOne((r) => r.params.has('cursor'))
        .flush('boom', { status: 500, statusText: 'Internal Server Error' })
      fixture.detectChanges()

      expect(fixture.componentInstance.entries()).toHaveLength(1)
      expect(fixture.nativeElement.textContent).toContain('Impossible de charger la suite')
      expect(loadMoreButton(fixture.nativeElement)?.disabled).toBe(false)

      fixture.componentInstance.loadMore()
      httpMock.expectOne((r) => r.params.get('cursor') === 'c1').flush(page([entry('2')]))
      fixture.detectChanges()

      expect(fixture.componentInstance.entries()).toHaveLength(2)
      expect(fixture.nativeElement.textContent).not.toContain('Impossible de charger la suite')
    })

    it('drops a next page that arrives after the organization changed', () => {
      const { fixture, httpMock } = render()
      httpMock.expectOne((r) => r.url === '/api/audit/events').flush(page([entry('1')], 'c1'))
      fixture.componentInstance.loadMore()
      const late = httpMock.expectOne((r) => r.params.has('cursor'))

      fixture.componentRef.setInput('organizationId', 'org-2')
      fixture.detectChanges()
      httpMock
        .expectOne((r) => r.params.get('organization_id') === 'org-2')
        .flush(page([entry('9')]))
      late.flush(page([entry('2')], 'c2'))

      expect(fixture.componentInstance.entries().map((e) => e.aggregate_id)).toEqual(['9'])
      expect(fixture.componentInstance.nextCursor()).toBeNull()
    })
  })

  it('shows the French name and details of an administrative event', () => {
    TestBed.configureTestingModule({
      providers: [provideHttpClient(), provideHttpClientTesting(), ...adminProviders],
    })
    const fixture = TestBed.createComponent(AuditLog)
    const httpMock = TestBed.inject(HttpTestingController)
    fixture.detectChanges()

    httpMock
      .expectOne((r) => r.url === '/api/audit/events')
      .flush(
        page([
          {
            aggregate_type: 'Admin',
            aggregate_id: 'a1',
            event_type: 'OrganizationCreated',
            payload: { slug: 'acme', display_name: 'Acme Corp' },
            occurred_at: '2026-01-01T00:00:00Z',
            actor_id: 'u1',
          },
        ]),
      )
    fixture.detectChanges()

    const text = fixture.nativeElement.textContent as string
    expect(text).toContain('Organisation créée')
    expect(text).toContain('Acme Corp (acme)')
    expect(text).not.toContain('OrganizationCreated')
  })

  describe('CSV export of a partly loaded log', () => {
    // Earlier tests in the file leave their URL spies behind.
    beforeEach(() => vi.restoreAllMocks())
    afterEach(() => vi.restoreAllMocks())

    const entry = (n: number, eventType = 'QuotaSet') => ({
      aggregate_type: 'Admin',
      aggregate_id: `agg-${n}`,
      event_type: eventType,
      payload: {},
      occurred_at: '2026-01-01T00:00:00Z',
      actor_id: null,
    })
    const range = (from: number, count: number) =>
      Array.from({ length: count }, (_, i) => entry(from + i))

    function render(loaded: unknown[], nextCursor: string | null) {
      TestBed.configureTestingModule({
        providers: [provideHttpClient(), provideHttpClientTesting(), ...adminProviders],
      })
      const fixture = TestBed.createComponent(AuditLog)
      const httpMock = TestBed.inject(HttpTestingController)
      const createObjectURL = vi.spyOn(URL, 'createObjectURL').mockReturnValue('blob:mock')
      vi.spyOn(URL, 'revokeObjectURL').mockImplementation(() => undefined)
      fixture.detectChanges()
      httpMock.expectOne((r) => r.url === '/api/audit/events').flush(page(loaded, nextCursor))
      fixture.detectChanges()
      return { fixture, httpMock, createObjectURL }
    }

    const exportedCsv = async (createObjectURL: { mock: { calls: unknown[][] } }) =>
      (createObjectURL.mock.calls[0][0] as Blob).text()

    it('fetches the pages that are not loaded yet, so the file holds the whole log', async () => {
      const { fixture, httpMock, createObjectURL } = render(range(0, 2), 'c1')

      fixture.componentInstance.downloadCsv()
      const first = httpMock.expectOne((r) => r.params.get('cursor') === 'c1')
      expect(first.request.params.get('exclude_aggregate_type')).toBe('Security')
      first.flush(page(range(2, 2), 'c2'))
      httpMock.expectOne((r) => r.params.get('cursor') === 'c2').flush(page(range(4, 1)))

      const csv = await exportedCsv(createObjectURL)
      expect(csv.split('\r\n')).toHaveLength(1 + 5)
      expect(csv).toContain('agg-4')
      expect(fixture.componentInstance.exportPartial()).toBe(false)
      expect(fixture.componentInstance.exporting()).toBe(false)
    })

    it('exports at once, without extra requests, when everything is loaded', async () => {
      const { fixture, httpMock, createObjectURL } = render(range(0, 2), null)

      fixture.componentInstance.downloadCsv()

      httpMock.expectNone((r) => r.params.has('cursor'))
      expect((await exportedCsv(createObjectURL)).split('\r\n')).toHaveLength(1 + 2)
    })

    it('shows progress and a cancel button while pages are being fetched', () => {
      const { fixture, httpMock } = render(range(0, 2), 'c1')

      fixture.componentInstance.downloadCsv()
      httpMock.expectOne((r) => r.params.get('cursor') === 'c1').flush(page(range(2, 3), 'c2'))
      fixture.detectChanges()

      const text = fixture.nativeElement.textContent as string
      expect(text).toContain('Export en cours… 5 entrées récupérées')
      expect(text).toContain('Annuler')
      expect(text).not.toContain('Télécharger en CSV')
      httpMock.expectOne((r) => r.params.get('cursor') === 'c2')
    })

    it('produces no file and drops the pending request when the export is cancelled', () => {
      const { fixture, httpMock, createObjectURL } = render(range(0, 2), 'c1')

      fixture.componentInstance.downloadCsv()
      const pending = httpMock.expectOne((r) => r.params.get('cursor') === 'c1')
      fixture.componentInstance.cancelExport()
      fixture.detectChanges()

      expect(pending.cancelled).toBe(true)
      expect(createObjectURL).not.toHaveBeenCalled()
      expect(fixture.componentInstance.exporting()).toBe(false)
      expect(fixture.nativeElement.textContent).toContain('Télécharger en CSV')
    })

    it('produces no file and says so when a page fails midway', () => {
      const { fixture, httpMock, createObjectURL } = render(range(0, 2), 'c1')

      fixture.componentInstance.downloadCsv()
      httpMock
        .expectOne((r) => r.params.get('cursor') === 'c1')
        .flush({}, { status: 500, statusText: 'Error' })
      fixture.detectChanges()

      expect(createObjectURL).not.toHaveBeenCalled()
      expect(fixture.componentInstance.exportFailed()).toBe(true)
      expect(fixture.nativeElement.querySelector('[role="alert"]')?.textContent).toContain(
        "L'export a échoué",
      )
    })

    it('cuts the file at the row bound and labels it partial', async () => {
      const { fixture, httpMock, createObjectURL } = render(range(0, 100), 'c1')

      fixture.componentInstance.downloadCsv()
      httpMock.expectOne((r) => r.params.get('cursor') === 'c1').flush(page(range(100, 5000), 'c2'))
      httpMock
        .expectOne((r) => r.params.get('cursor') === 'c2')
        .flush(page(range(5100, 5000), 'c3'))
      httpMock.expectNone((r) => r.params.get('cursor') === 'c3')
      fixture.detectChanges()

      const csv = await exportedCsv(createObjectURL)
      expect(csv.split('\r\n')).toHaveLength(1 + 10_000)
      expect(fixture.componentInstance.exportPartial()).toBe(true)
      expect(fixture.nativeElement.textContent).toContain('10000 entrées les plus récentes')
    })

    it('tells the reader what the export and the summary cover while more entries exist', () => {
      const { fixture } = render(range(0, 3), 'c1')

      const text = fixture.nativeElement.textContent as string
      expect(text).toContain('Calculé sur les 3 entrées chargées')
      expect(text).toContain("3 entrées chargées ; l'export récupère aussi les suivantes")
    })

    it('shows no partial-coverage note once everything is loaded', () => {
      const { fixture } = render(range(0, 3), null)

      const text = fixture.nativeElement.textContent as string
      expect(text).not.toContain('Calculé sur')
      expect(text).not.toContain("l'export récupère")
    })

    it('stops an export in progress when the organization changes', () => {
      const { fixture, httpMock, createObjectURL } = render(range(0, 2), 'c1')

      fixture.componentInstance.downloadCsv()
      const pending = httpMock.expectOne((r) => r.params.get('cursor') === 'c1')
      fixture.componentRef.setInput('organizationId', 'org-2')
      fixture.detectChanges()

      expect(pending.cancelled).toBe(true)
      expect(createObjectURL).not.toHaveBeenCalled()
      httpMock.expectOne((r) => r.params.get('organization_id') === 'org-2')
    })
  })
})
