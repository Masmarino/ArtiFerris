import { TestBed } from '@angular/core/testing'
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing'
import { provideHttpClient } from '@angular/common/http'
import { signal } from '@angular/core'
import { HealthStatusPage } from './health-status'
import { adminProviders } from '../infrastructure/admin.providers'
import { PageTitleService } from '../../shell/page-title.service'
import { ToastService } from '../../shared/toast.service'

const HEALTH = {
  database: {
    status: 'up',
    detail: null,
    response_time_ms: 4,
    active_connections: 3,
    max_connections: 10,
    server_version: '18.0',
  },
  storage: {
    status: 'down',
    detail: 'disk full',
    used_bytes: 900,
    free_bytes: 100,
    total_bytes: 1000,
  },
  uptime_seconds: 90_000,
}

describe('HealthStatusPage', () => {
  function setup() {
    TestBed.configureTestingModule({
      providers: [
        provideHttpClient(),
        provideHttpClientTesting(),
        ...adminProviders,
        { provide: PageTitleService, useValue: { title: signal('Santé') } },
      ],
    })
    const fixture = TestBed.createComponent(HealthStatusPage)
    const httpMock = TestBed.inject(HttpTestingController)
    fixture.detectChanges()
    return { fixture, httpMock, element: fixture.nativeElement as HTMLElement }
  }

  function render(health: object = HEALTH) {
    const context = setup()
    context.httpMock.expectOne('/api/admin/health').flush(health)
    context.fixture.detectChanges()
    return context
  }

  it('checks the services on opening, with a placeholder until they answer', () => {
    const { element, httpMock } = setup()

    expect(element.querySelector('[role="status"]')?.textContent).toContain(
      'Vérification des services…',
    )
    httpMock.expectOne('/api/admin/health')
  })

  it('names each service state in words, not as raw values', () => {
    const { element } = render()
    const cards = element.querySelectorAll('.health__card')

    expect(cards[0].textContent).toContain('Opérationnelle')
    expect(cards[1].textContent).toContain('Indisponible')
    expect(cards[1].textContent).toContain('disk full')
    expect(element.textContent).not.toContain(' up ')
  })

  it("sums up the instance's state in the header", () => {
    const { element } = render()

    expect(element.textContent).toContain('Service dégradé')
  })

  it('says all is well when every service is up', () => {
    const { element } = render({
      ...HEALTH,
      storage: { ...HEALTH.storage, status: 'up', detail: null },
    })

    expect(element.textContent).toContain('Tous les services sont opérationnels')
    // French puts a narrow no-break space before the percent sign.
    expect(element.textContent).toMatch(/900 o sur 1000 o · 90\s%/)
  })

  it('shows the database version, response time and connections', () => {
    const { element } = render()
    const database = element.querySelectorAll('.health__card')[0].textContent

    expect(database).toContain('PostgreSQL 18.0')
    expect(database).toContain('4 ms')
    expect(database).toContain('3 sur 10')
    const fill = element.querySelector<HTMLElement>('.health__card .gbt-gauge-bar__fill')
    expect(fill?.style.width).toBe('30%')
  })

  it('shows the uptime and when the server started on the side', () => {
    const { element } = render()
    const aside = element.querySelector('.health__aside')?.textContent

    expect(aside).toContain('1 j')
    expect(aside).toContain('Démarré le')
  })

  it('checks again on "Actualiser"', () => {
    const { fixture, httpMock } = render()

    fixture.componentInstance.refresh()
    expect(fixture.componentInstance.checking()).toBe(true)
    httpMock.expectOne('/api/admin/health').flush(HEALTH)

    expect(fixture.componentInstance.checking()).toBe(false)
  })

  it('says the API is unreachable when the check itself fails', () => {
    const { fixture, httpMock, element } = setup()
    const toast = vi.spyOn(TestBed.inject(ToastService), 'error')

    httpMock.expectOne('/api/admin/health').flush('boom', { status: 500, statusText: 'Error' })
    fixture.detectChanges()

    expect(element.textContent).toContain('Injoignable')
    expect(element.textContent).toContain('API ou base de données injoignable.')
    expect(element.querySelector('.health__aside')).toBeNull()
    expect(toast).toHaveBeenCalled()
  })
})
