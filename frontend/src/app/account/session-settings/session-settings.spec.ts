import { TestBed } from '@angular/core/testing'
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing'
import { provideHttpClient } from '@angular/common/http'
import { SessionSettings } from './session-settings'
import { authProviders } from '../../auth/infrastructure/auth.providers'
import { SessionRevocationService } from '../../auth/application/session-revocation.service'
import { ConfirmService } from '../../shared/confirm.service'
import { ToastService } from '../../shared/toast.service'

describe('SessionSettings', () => {
  const signOutAndRedirect = vi.fn()
  const ask = vi.fn()

  function render(confirmed: boolean) {
    signOutAndRedirect.mockClear()
    ask.mockReset().mockResolvedValue(confirmed)
    TestBed.configureTestingModule({
      providers: [
        provideHttpClient(),
        provideHttpClientTesting(),
        ...authProviders,
        { provide: SessionRevocationService, useValue: { signOutAndRedirect } },
        { provide: ConfirmService, useValue: { ask } },
      ],
    })
    const fixture = TestBed.createComponent(SessionSettings)
    fixture.detectChanges()
    return { fixture, httpMock: TestBed.inject(HttpTestingController) }
  }

  it('offers the action', () => {
    const { fixture } = render(true)

    expect(fixture.nativeElement.textContent).toContain('Se déconnecter partout')
  })

  it('asks for confirmation, calls logout-all, then signs out with the revoked-sessions reason', async () => {
    const { fixture, httpMock } = render(true)

    const done = fixture.componentInstance.logoutEverywhere()
    await Promise.resolve()

    expect(ask).toHaveBeenCalledWith(expect.objectContaining({ danger: true }))
    const req = httpMock.expectOne('/api/auth/logout-all')
    expect(req.request.method).toBe('POST')
    expect(signOutAndRedirect).not.toHaveBeenCalled()
    req.flush(null, { status: 204, statusText: 'No Content' })
    await done

    expect(signOutAndRedirect).toHaveBeenCalledTimes(1)
  })

  it('does nothing when the confirmation is dismissed', async () => {
    const { fixture, httpMock } = render(false)

    await fixture.componentInstance.logoutEverywhere()

    httpMock.expectNone('/api/auth/logout-all')
    expect(signOutAndRedirect).not.toHaveBeenCalled()
  })

  it('keeps the session and reports the failure when the server call fails', async () => {
    const { fixture, httpMock } = render(true)

    const done = fixture.componentInstance.logoutEverywhere()
    await Promise.resolve()
    httpMock
      .expectOne('/api/auth/logout-all')
      .flush(null, { status: 500, statusText: 'Server Error' })
    await done

    expect(signOutAndRedirect).not.toHaveBeenCalled()
    expect(fixture.componentInstance.signingOut()).toBe(false)
    expect(TestBed.inject(ToastService).toasts().at(-1)).toMatchObject({
      variant: 'error',
      message: 'Impossible de fermer les sessions. Réessayez.',
    })
  })

  it('ignores a second click while the request is in flight', async () => {
    const { fixture, httpMock } = render(true)

    void fixture.componentInstance.logoutEverywhere()
    await Promise.resolve()
    await fixture.componentInstance.logoutEverywhere()

    expect(ask).toHaveBeenCalledTimes(1)
    httpMock
      .expectOne('/api/auth/logout-all')
      .flush(null, { status: 204, statusText: 'No Content' })
  })
})
