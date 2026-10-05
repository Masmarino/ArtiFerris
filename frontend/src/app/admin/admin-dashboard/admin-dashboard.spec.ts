import { TestBed } from '@angular/core/testing'
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing'
import { provideHttpClient } from '@angular/common/http'
import { provideRouter } from '@angular/router'
import { formatBytes } from '../../shared/format'
import { AdminDashboard } from './admin-dashboard'
import { repositoryProviders } from '../../repositories/infrastructure/repository.providers'
import { adminProviders } from '../infrastructure/admin.providers'

describe('AdminDashboard', () => {
  function render() {
    TestBed.configureTestingModule({
      providers: [
        provideHttpClient(),
        provideHttpClientTesting(),
        provideRouter([]),
        ...repositoryProviders,
        ...adminProviders,
      ],
    })
    const fixture = TestBed.createComponent(AdminDashboard)
    const httpMock = TestBed.inject(HttpTestingController)
    fixture.detectChanges()
    return { fixture, httpMock }
  }

  function flushCommon(
    httpMock: HttpTestingController,
    stats = { total_users: 0, total_repositories: 0, total_active_permissions: 0 },
    activityEntries: unknown[] = [],
    repositories: unknown[] = [],
    history: unknown[] = [],
  ) {
    httpMock.expectOne('/api/admin/stats').flush(stats)
    httpMock
      .expectOne((r) => r.url === '/api/audit/events' && r.params.has('from'))
      .flush({ entries: activityEntries, next_cursor: null })
    httpMock.expectOne('/api/repositories').flush(repositories)
    // One period for both charts, so one request.
    httpMock.expectOne((r) => r.url === '/api/admin/metrics/history').flush(history)
  }

  it('asks for the largest audit page so the activity chart keeps its window', () => {
    const { httpMock } = render()

    const request = httpMock.expectOne((r) => r.url === '/api/audit/events' && r.params.has('from'))

    expect(request.request.params.get('limit')).toBe('200')
    httpMock.match(() => true)
  })

  it('shows an error with a retry when a load fails, and clears it once a retry succeeds', () => {
    const { fixture, httpMock } = render()
    httpMock.expectOne('/api/admin/stats').flush(null, { status: 500, statusText: 'Server Error' })
    httpMock
      .expectOne((r) => r.url === '/api/audit/events' && r.params.has('from'))
      .flush({ entries: [], next_cursor: null })
    httpMock.expectOne('/api/repositories').flush([])
    httpMock.match((r) => r.url === '/api/admin/metrics/history').forEach((req) => req.flush([]))
    fixture.detectChanges()

    const alert = fixture.nativeElement.querySelector('[role="alert"]')
    expect(alert.textContent).toContain("Une partie du tableau de bord n'a pas pu être chargée.")

    fixture.componentInstance.retry()
    // The repository list succeeded the first time and is cached, so only the rest is refetched.
    httpMock.expectOne('/api/admin/stats').flush({
      total_users: 0,
      total_repositories: 0,
      total_active_permissions: 0,
    })
    httpMock
      .expectOne((r) => r.url === '/api/audit/events' && r.params.has('from'))
      .flush({ entries: [], next_cursor: null })
    httpMock.match((r) => r.url === '/api/admin/metrics/history').forEach((req) => req.flush([]))
    fixture.detectChanges()

    expect(fixture.nativeElement.querySelector('[role="alert"]')).toBeNull()
  })

  it('reports a failed history load too', () => {
    const { fixture, httpMock } = render()
    httpMock.expectOne('/api/admin/stats').flush({
      total_users: 0,
      total_repositories: 0,
      total_active_permissions: 0,
    })
    httpMock
      .expectOne((r) => r.url === '/api/audit/events' && r.params.has('from'))
      .flush({ entries: [], next_cursor: null })
    httpMock.expectOne('/api/repositories').flush([])
    httpMock
      .match((r) => r.url === '/api/admin/metrics/history')
      .forEach((req) => req.flush(null, { status: 500, statusText: 'Server Error' }))
    fixture.detectChanges()

    expect(fixture.componentInstance.loadFailed()).toBe(true)
  })

  it('shows the French label of a recent event, not its raw type', () => {
    const { fixture, httpMock } = render()

    flushCommon(httpMock, undefined, [
      {
        aggregate_type: 'Security',
        aggregate_id: 'x',
        event_type: 'LoginFailed',
        payload: {},
        occurred_at: '2026-01-01T00:00:00Z',
        actor_id: null,
      },
    ])
    fixture.detectChanges()

    const text = fixture.nativeElement.textContent as string
    expect(text).toContain('Échec de connexion')
    expect(text).not.toContain('LoginFailed')
  })

  it('shows the totals as tiles, with the storage of the last reading', () => {
    const { fixture, httpMock } = render()

    flushCommon(
      httpMock,
      { total_users: 5, total_repositories: 3, total_active_permissions: 7 },
      [],
      [],
      [
        {
          recorded_at: '2026-01-01T10:00:00Z',
          total_users: 5,
          total_repositories: 3,
          total_storage_bytes: 2048,
        },
      ],
    )
    fixture.detectChanges()

    const tile = (key: string) =>
      (fixture.nativeElement as HTMLElement).querySelector(`[data-key="${key}"]`)?.textContent
    expect(tile('users')).toContain('5')
    expect(tile('repositories')).toContain('3')
    expect(tile('permissions')).toContain('7')
    expect(tile('storage')).toContain('2,0 Ko')
    expect(tile('storage')).toContain('dernier relevé')
  })

  it('says there are not enough readings for a line with fewer than two', () => {
    const { fixture, httpMock } = render()

    flushCommon(httpMock)
    fixture.detectChanges()

    expect(fixture.nativeElement.textContent).toContain('Pas encore assez de mesures')
    expect(fixture.nativeElement.querySelector('gbt-line-chart')).toBeNull()
  })

  it('shows at most the 10 most recent activity entries', () => {
    const { fixture, httpMock } = render()
    const entries = Array.from({ length: 15 }, (_, i) => ({
      aggregate_type: 'PackageRepository',
      aggregate_id: `repo-${i}`,
      event_type: 'Created',
      payload: {},
      occurred_at: `2026-01-01T00:00:${String(i).padStart(2, '0')}Z`,
      actor_id: null,
    }))

    flushCommon(httpMock, undefined, entries)
    fixture.detectChanges()

    expect(fixture.componentInstance.recentEvents().length).toBe(10)
    expect(fixture.componentInstance.activityEvents().length).toBe(15)
  })

  it('shows an empty-state message when there is no recent activity', () => {
    const { fixture, httpMock } = render()

    flushCommon(httpMock)
    fixture.detectChanges()

    expect(fixture.nativeElement.textContent).toContain('Aucune activité récente')
  })

  it('buckets activity events by day into a 7-day chart, zero-filling quiet days', () => {
    const { fixture, httpMock } = render()
    const today = new Date().toISOString().slice(0, 10)

    flushCommon(httpMock, undefined, [
      {
        aggregate_type: 'PackageRepository',
        aggregate_id: 'a',
        event_type: 'Created',
        payload: {},
        occurred_at: `${today}T12:00:00Z`,
        actor_id: null,
      },
      {
        aggregate_type: 'PackageRepository',
        aggregate_id: 'b',
        event_type: 'Created',
        payload: {},
        occurred_at: `${today}T13:00:00Z`,
        actor_id: null,
      },
    ])
    fixture.detectChanges()

    const data = fixture.componentInstance.activityChartData()
    expect(data.length).toBe(7)
    expect(data[data.length - 1].value).toBe(2)
    expect(data.slice(0, 6).every((d) => d.value === 0)).toBe(true)
  })

  it('groups repositories by format for the repositories chart', () => {
    const { fixture, httpMock } = render()

    flushCommon(
      httpMock,
      undefined,
      [],
      [
        {
          id: '1',
          name: 'a',
          format: 'npm',
          repo_type: 'hosted',
          remote_url: null,
          group_members: [],
        },
        {
          id: '2',
          name: 'b',
          format: 'npm',
          repo_type: 'hosted',
          remote_url: null,
          group_members: [],
        },
        {
          id: '3',
          name: 'c',
          format: 'docker',
          repo_type: 'hosted',
          remote_url: null,
          group_members: [],
        },
      ],
    )
    fixture.detectChanges()

    expect(fixture.componentInstance.repositoriesByFormatChartData()).toEqual(
      expect.arrayContaining([
        { label: 'npm', value: 2 },
        { label: 'docker', value: 1 },
      ]),
    )
  })

  it('builds evolution chart series from the metrics history', () => {
    const { fixture, httpMock } = render()

    flushCommon(
      httpMock,
      undefined,
      [],
      [],
      [
        {
          recorded_at: '2026-01-01T10:00:00Z',
          total_users: 1,
          total_repositories: 2,
          total_storage_bytes: 1024,
        },
        {
          recorded_at: '2026-01-01T11:00:00Z',
          total_users: 2,
          total_repositories: 2,
          total_storage_bytes: 2048,
        },
      ],
    )
    fixture.detectChanges()

    // In the largest reading's unit, so the axis reads 1 and 2 (Ko).
    expect(fixture.componentInstance.storageSeries()).toEqual([
      {
        label: 'Stockage (Ko)',
        points: [
          { x: new Date('2026-01-01T10:00:00Z'), y: 1, display: formatBytes(1024) },
          { x: new Date('2026-01-01T11:00:00Z'), y: 2, display: formatBytes(2048) },
        ],
      },
    ])
    expect(fixture.componentInstance.countsSeries()).toEqual([
      {
        label: 'Utilisateurs',
        points: [
          { x: new Date('2026-01-01T10:00:00Z'), y: 1 },
          { x: new Date('2026-01-01T11:00:00Z'), y: 2 },
        ],
      },
      {
        label: 'Dépôts',
        points: [
          { x: new Date('2026-01-01T10:00:00Z'), y: 2 },
          { x: new Date('2026-01-01T11:00:00Z'), y: 2 },
        ],
      },
    ])
  })

  it('shows 30 days by default, and one period change refetches both charts', () => {
    const { fixture, httpMock } = render()
    flushCommon(httpMock)
    fixture.detectChanges()

    expect(fixture.componentInstance.historyDays()).toBe(30)

    fixture.componentInstance.setHistoryDays(7)

    const req = httpMock.expectOne((r) => r.url === '/api/admin/metrics/history')
    expect(req.request.params.get('days')).toBe('7')
    req.flush([])
    expect(fixture.componentInstance.historyDays()).toBe(7)
  })

  it('drops a slower answer for an earlier period', () => {
    const { fixture, httpMock } = render()
    flushCommon(httpMock)

    fixture.componentInstance.setHistoryDays(1)
    const first = httpMock.expectOne((r) => r.params.get('days') === '1')
    fixture.componentInstance.setHistoryDays(3)
    const second = httpMock.expectOne((r) => r.params.get('days') === '3')

    expect(first.cancelled).toBe(true)
    second.flush([])
  })

  describe('activity chart coverage', () => {
    const entry = (n: number) => ({
      aggregate_type: 'Admin',
      aggregate_id: `a${n}`,
      event_type: 'QuotaSet',
      payload: {},
      occurred_at: new Date().toISOString(),
      actor_id: null,
    })

    function renderWithActivity(nextCursor: string | null) {
      const { fixture, httpMock } = render()
      httpMock.expectOne('/api/admin/stats').flush({
        total_users: 0,
        total_repositories: 0,
        total_active_permissions: 0,
      })
      httpMock
        .expectOne((r) => r.url === '/api/audit/events' && r.params.has('from'))
        .flush({ entries: [entry(1), entry(2), entry(3)], next_cursor: nextCursor })
      httpMock.expectOne('/api/repositories').flush([])
      httpMock.expectOne((r) => r.url === '/api/admin/metrics/history').flush([])
      fixture.detectChanges()
      return fixture.nativeElement as HTMLElement
    }

    it('warns that older days are under-counted when the server has more events', () => {
      const el = renderWithActivity('more')

      expect(el.textContent).toContain('Basé sur les 3 événements les plus récents')
    })

    it('shows no warning when the whole week fits in the fetched page', () => {
      const el = renderWithActivity(null)

      expect(el.textContent).not.toContain('Basé sur les')
    })
  })
})
