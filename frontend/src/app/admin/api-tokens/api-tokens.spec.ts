import { TestBed } from '@angular/core/testing'
import { By } from '@angular/platform-browser'
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing'
import { provideHttpClient } from '@angular/common/http'
import { Tooltip } from '@masmarino/gabarit/tooltip'
import { ApiTokensAdmin } from './api-tokens'
import { adminProviders } from '../infrastructure/admin.providers'
import { ConfirmService } from '../../shared/confirm.service'
import { ToastService } from '../../shared/toast.service'

function render(confirmed = true) {
  const ask = vi.fn().mockResolvedValue(confirmed)
  TestBed.configureTestingModule({
    providers: [
      provideHttpClient(),
      provideHttpClientTesting(),
      ...adminProviders,
      { provide: ConfirmService, useValue: { ask } },
    ],
  })
  const fixture = TestBed.createComponent(ApiTokensAdmin)
  const httpMock = TestBed.inject(HttpTestingController)
  fixture.detectChanges()
  return { fixture, httpMock, ask }
}

function flushTokens(httpMock: HttpTestingController, tokens: unknown[]) {
  httpMock.expectOne('/api/admin/tokens?limit=500').flush(tokens)
}

describe('ApiTokensAdmin', () => {
  afterEach(() => {
    vi.restoreAllMocks()
  })

  it('shows an empty message when there are no active tokens', () => {
    const { fixture, httpMock } = render()
    flushTokens(httpMock, [])
    fixture.detectChanges()

    expect(fixture.nativeElement.textContent).toContain('Aucun jeton actif')
  })

  it('splits tokens into active and revoked sections', () => {
    const { fixture, httpMock } = render()
    flushTokens(httpMock, [
      {
        id: 't1',
        user_id: 'u1',
        username: 'florian',
        label: 'laptop',
        created_at: '2026-01-01T00:00:00Z',
        last_used_at: null,
        revoked_at: null,
      },
      {
        id: 't2',
        user_id: 'u2',
        username: 'bob',
        label: 'ci',
        created_at: '2026-01-01T00:00:00Z',
        last_used_at: null,
        revoked_at: '2026-01-02T00:00:00Z',
      },
    ])
    fixture.detectChanges()

    expect(fixture.componentInstance.activeTokens().map((t) => t.id)).toEqual(['t1'])
    expect(fixture.componentInstance.revokedTokens().map((t) => t.id)).toEqual(['t2'])
    const text = fixture.nativeElement.textContent as string
    expect(text).toContain('Jetons actifs')
    expect(text).toContain('Jetons révoqués')
    expect(text).toContain('florian')
    expect(text).toContain('bob')
  })

  it('does not show the revoked section when there are no revoked tokens', () => {
    const { fixture, httpMock } = render()
    flushTokens(httpMock, [
      {
        id: 't1',
        user_id: 'u1',
        username: 'florian',
        label: 'laptop',
        created_at: '2026-01-01T00:00:00Z',
        last_used_at: null,
        revoked_at: null,
      },
    ])
    fixture.detectChanges()

    expect(fixture.nativeElement.textContent).not.toContain('Jetons révoqués')
  })

  it('revokes a token after confirmation and reloads the list', async () => {
    const { fixture, httpMock, ask } = render()
    flushTokens(httpMock, [
      {
        id: 't1',
        user_id: 'u1',
        username: 'florian',
        label: 'laptop',
        created_at: '2026-01-01T00:00:00Z',
        last_used_at: null,
        revoked_at: null,
      },
    ])
    fixture.detectChanges()

    const button: HTMLButtonElement = fixture.nativeElement.querySelector(
      '.api-tokens__actions button',
    )
    button.click()
    await fixture.whenStable()

    expect(ask).toHaveBeenCalledWith(
      expect.objectContaining({
        heading: 'Révoquer le jeton',
        message: 'Révoquer le jeton « laptop » de florian ?',
      }),
    )
    const revokeReq = httpMock.expectOne('/api/admin/tokens/t1')
    expect(revokeReq.request.method).toBe('DELETE')
    revokeReq.flush(null)

    flushTokens(httpMock, [])

    const toastService = TestBed.inject(ToastService)
    expect(toastService.toasts().at(-1)).toMatchObject({
      variant: 'success',
      message: 'Jeton « laptop » révoqué.',
    })
  })

  it('does not revoke when the confirmation is cancelled', async () => {
    const { fixture, httpMock, ask } = render(false)
    flushTokens(httpMock, [
      {
        id: 't1',
        user_id: 'u1',
        username: 'florian',
        label: 'laptop',
        created_at: '2026-01-01T00:00:00Z',
        last_used_at: null,
        revoked_at: null,
      },
    ])
    fixture.detectChanges()

    const button: HTMLButtonElement = fixture.nativeElement.querySelector(
      '.api-tokens__actions button',
    )
    button.click()
    await fixture.whenStable()

    expect(ask).toHaveBeenCalled()
    httpMock.expectNone('/api/admin/tokens/t1')
  })

  it('re-fetches when organizationId changes to a different organization — the component is reused, not recreated, across a super-admin switching organizations', () => {
    const { fixture, httpMock } = render()
    httpMock.expectOne('/api/admin/tokens?limit=500').flush([])
    fixture.componentRef.setInput('organizationId', 'org-1')
    fixture.detectChanges()
    httpMock.expectOne('/api/admin/tokens?organization_id=org-1&limit=500').flush([
      {
        id: 't1',
        user_id: 'u1',
        username: 'org1-user',
        label: 'laptop',
        created_at: '2026-01-01T00:00:00Z',
        last_used_at: null,
        revoked_at: null,
      },
    ])
    fixture.detectChanges()
    expect(fixture.nativeElement.textContent).toContain('org1-user')

    fixture.componentRef.setInput('organizationId', 'org-2')
    fixture.detectChanges()
    httpMock.expectOne('/api/admin/tokens?organization_id=org-2&limit=500').flush([])
    fixture.detectChanges()

    expect(fixture.nativeElement.textContent).not.toContain('org1-user')
  })

  it('explains via a tooltip that revoking a token is immediate and permanent', () => {
    const { fixture, httpMock } = render()
    flushTokens(httpMock, [
      {
        id: 't1',
        user_id: 'u1',
        username: 'florian',
        label: 'laptop',
        created_at: '2026-01-01T00:00:00Z',
        last_used_at: null,
        revoked_at: null,
      },
    ])
    fixture.detectChanges()

    const tooltip = fixture.debugElement.query(By.directive(Tooltip))

    expect((tooltip.componentInstance as Tooltip).text()).toBe(
      'Le jeton cessera immédiatement de fonctionner, définitivement.',
    )
  })

  it('keeps the current organization when a slower response for the previous one lands last', () => {
    const { fixture, httpMock } = render()
    httpMock.expectOne('/api/admin/tokens?limit=500').flush([])
    const token = (username: string) => ({
      id: username,
      user_id: username,
      username,
      label: 'laptop',
      created_at: '2026-01-01T00:00:00Z',
      last_used_at: null,
      revoked_at: null,
    })
    fixture.componentRef.setInput('organizationId', 'org-a')
    fixture.detectChanges()
    const forA = httpMock.expectOne('/api/admin/tokens?organization_id=org-a&limit=500')
    fixture.componentRef.setInput('organizationId', 'org-b')
    fixture.detectChanges()
    const forB = httpMock.expectOne('/api/admin/tokens?organization_id=org-b&limit=500')

    forB.flush([token('b-user')])
    forA.flush([token('a-user')])
    fixture.detectChanges()

    expect(fixture.nativeElement.textContent).toContain('b-user')
    expect(fixture.nativeElement.textContent).not.toContain('a-user')
  })

  it('tells the admin when the list hit the 500-token page limit, revoked tokens included', () => {
    const { fixture, httpMock } = render()
    const token = (i: number, revoked: boolean) => ({
      id: `t${i}`,
      user_id: 'u1',
      username: 'florian',
      label: `key-${i}`,
      created_at: '2026-01-01T00:00:00Z',
      last_used_at: null,
      revoked_at: revoked ? '2026-02-01T00:00:00Z' : null,
    })
    // The limit counts every token returned, revoked ones included. Revoked rows are plain table
    // rows, while 500 active
    // ones build 500 buttons and tooltips: too slow when the machine is busy.
    flushTokens(
      httpMock,
      Array.from({ length: 500 }, (_, i) => token(i, i > 0)),
    )
    fixture.detectChanges()

    expect(fixture.nativeElement.textContent).toContain('La liste est limitée à 500 jetons')
  })

  it('shows no limit notice below the page limit', () => {
    const { fixture, httpMock } = render()
    flushTokens(httpMock, [])
    fixture.detectChanges()

    expect(fixture.nativeElement.textContent).not.toContain('La liste est limitée')
  })

  it('shows an error when the token list cannot be loaded', () => {
    const { fixture, httpMock } = render()

    httpMock
      .expectOne('/api/admin/tokens?limit=500')
      .flush(null, { status: 500, statusText: 'boom' })
    fixture.detectChanges()

    expect(fixture.nativeElement.textContent).toContain('Échec du chargement des jetons.')
  })
})
