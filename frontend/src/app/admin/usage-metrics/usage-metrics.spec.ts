import { ComponentFixture, TestBed } from '@angular/core/testing'
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing'
import { provideHttpClient } from '@angular/common/http'
import { UsageMetrics } from './usage-metrics'
import { adminProviders } from '../infrastructure/admin.providers'

async function render(
  usages: {
    repository_id: string
    name: string
    used_bytes: number
    quota_bytes?: number | null
  }[],
) {
  TestBed.configureTestingModule({
    providers: [provideHttpClient(), provideHttpClientTesting(), ...adminProviders],
  })
  const fixture = TestBed.createComponent(UsageMetrics)
  const httpMock = TestBed.inject(HttpTestingController)
  fixture.detectChanges()
  httpMock.expectOne('/api/admin/metrics').flush(usages)
  await settle(fixture)
  return fixture
}

async function settle(fixture: ComponentFixture<unknown>) {
  await fixture.whenStable()
  fixture.detectChanges()
}

describe('UsageMetrics', () => {
  it('renders one chart bar per repository, most-used first', async () => {
    const fixture = await render([
      { repository_id: '1', name: 'small-repo', used_bytes: 100 },
      { repository_id: '2', name: 'big-repo', used_bytes: 900 },
    ])

    const labels: (string | undefined)[] = Array.from(
      fixture.nativeElement.querySelectorAll('.gbt-dimension-card__label'),
    ).map((el: unknown) => (el as HTMLElement).textContent?.trim())

    expect(labels).toEqual(['big-repo', 'small-repo'])
  })

  it('folds repositories beyond the top 15 into one "Autres" bar', async () => {
    const usages = Array.from({ length: 17 }, (_, i) => ({
      repository_id: `${i}`,
      name: `repo-${i}`,
      used_bytes: 100 - i,
    }))
    const fixture = await render(usages)

    const rows = fixture.nativeElement.querySelectorAll('.gbt-dimension-card tbody tr')
    expect(rows.length).toBe(16)
    expect(fixture.nativeElement.textContent).toContain('Autres (2)')
  })

  it('still lists every repository in the table, including the folded ones', async () => {
    const usages = Array.from({ length: 17 }, (_, i) => ({
      repository_id: `${i}`,
      name: `repo-${i}`,
      used_bytes: 100 - i,
    }))
    const fixture = await render(usages)

    const tableRows = fixture.nativeElement.querySelectorAll('gbt-table tbody tr')
    expect(tableRows.length).toBe(17)
  })

  it('formats the chart values as bytes', async () => {
    const fixture = await render([{ repository_id: '1', name: 'repo', used_bytes: 2048 }])

    expect(fixture.nativeElement.textContent).toContain('2,0 Ko')
  })

  it('shows an empty-state message when there are no repositories', async () => {
    const fixture = await render([])

    expect(fixture.nativeElement.textContent).toContain('Aucun dépôt.')
  })

  it('shows a quota gauge only for repositories with a quota set', async () => {
    const fixture = await render([
      { repository_id: '1', name: 'limited-repo', used_bytes: 500, quota_bytes: 1000 },
      { repository_id: '2', name: 'unlimited-repo', used_bytes: 500, quota_bytes: null },
    ])

    const gauges = fixture.nativeElement.querySelectorAll('gbt-gauge-bar')
    expect(gauges.length).toBe(1)
    const text = fixture.nativeElement.textContent as string
    expect(text).toContain('limited-repo')
  })

  it('hides the quota section entirely when no repository has a quota', async () => {
    const fixture = await render([{ repository_id: '1', name: 'repo', used_bytes: 500 }])

    expect(fixture.nativeElement.textContent).not.toContain('Quotas de stockage')
  })

  describe('switching organization', () => {
    function renderScoped() {
      TestBed.configureTestingModule({
        providers: [provideHttpClient(), provideHttpClientTesting(), ...adminProviders],
      })
      const fixture = TestBed.createComponent(UsageMetrics)
      const httpMock = TestBed.inject(HttpTestingController)
      fixture.componentRef.setInput('organizationId', 'org-a')
      fixture.detectChanges()
      return { fixture, httpMock }
    }

    it("cancels organization A's request once organization B is on screen", async () => {
      const { fixture, httpMock } = renderScoped()
      const requestA = httpMock.expectOne('/api/admin/metrics?organization_id=org-a')

      fixture.componentRef.setInput('organizationId', 'org-b')
      fixture.detectChanges()
      const requestB = httpMock.expectOne('/api/admin/metrics?organization_id=org-b')
      requestB.flush([{ repository_id: 'b1', name: 'org-b-repo', used_bytes: 10 }])
      await settle(fixture)

      expect(requestA.cancelled).toBe(true)

      const text = fixture.nativeElement.textContent as string
      expect(text).toContain('org-b-repo')
      expect(text).not.toContain('org-a-repo')
    })

    it("drops organization A's rows while organization B loads", async () => {
      const { fixture, httpMock } = renderScoped()
      httpMock
        .expectOne('/api/admin/metrics?organization_id=org-a')
        .flush([{ repository_id: 'a1', name: 'org-a-repo', used_bytes: 10 }])
      await settle(fixture)
      expect(fixture.nativeElement.textContent).toContain('org-a-repo')

      fixture.componentRef.setInput('organizationId', 'org-b')
      fixture.detectChanges()

      expect(fixture.nativeElement.textContent).not.toContain('org-a-repo')
      httpMock.expectOne('/api/admin/metrics?organization_id=org-b')
    })

    it('shows an error with a retry instead of an unhandled failure, and recovers', async () => {
      const { fixture, httpMock } = renderScoped()
      httpMock
        .expectOne('/api/admin/metrics?organization_id=org-a')
        .flush({}, { status: 500, statusText: 'Error' })
      await settle(fixture)

      const el = fixture.nativeElement as HTMLElement
      expect(el.querySelector('[role="alert"]')).toBeTruthy()

      const retry = Array.from(el.querySelectorAll('button')).find((b) =>
        b.textContent?.includes('Réessayer'),
      )!
      retry.click()
      fixture.detectChanges()
      httpMock
        .expectOne('/api/admin/metrics?organization_id=org-a')
        .flush([{ repository_id: 'a1', name: 'org-a-repo', used_bytes: 10 }])
      await settle(fixture)

      expect(el.querySelector('[role="alert"]')).toBeNull()
      expect(el.textContent).toContain('org-a-repo')
    })
  })
})
