import { TestBed } from '@angular/core/testing'
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing'
import { provideHttpClient } from '@angular/common/http'
import { ActivatedRoute, Router, convertToParamMap, provideRouter } from '@angular/router'
import { ActivatePage } from './activate-page'
import { authProviders } from '../infrastructure/auth.providers'

function render(token: string | null = 'raw-token') {
  TestBed.configureTestingModule({
    imports: [ActivatePage],
    providers: [
      provideHttpClient(),
      provideHttpClientTesting(),
      provideRouter([]),
      ...authProviders,
      {
        provide: ActivatedRoute,
        useValue: {
          snapshot: { queryParamMap: convertToParamMap(token === null ? {} : { token }) },
        },
      },
    ],
  })
  const fixture = TestBed.createComponent(ActivatePage)
  const httpMock = TestBed.inject(HttpTestingController)
  fixture.detectChanges()
  return { fixture, httpMock }
}

describe('ActivatePage', () => {
  afterEach(() => {
    TestBed.inject(HttpTestingController).verify()
    vi.restoreAllMocks()
  })

  it('shows an error and no form when the token query param is missing', () => {
    const { fixture } = render(null)

    expect(fixture.componentInstance.tokenMissing).toBe(true)
    expect(fixture.nativeElement.textContent).toContain('Lien')
  })

  it('rejects a mismatched password confirmation without sending a request', () => {
    const { fixture, httpMock } = render()
    fixture.componentInstance.form.setValue({
      username: 'florian',
      newPassword: 'new-s3cret!',
      confirmPassword: 'different!',
    })

    fixture.componentInstance.submit()

    httpMock.expectNone('/api/auth/activate')
    expect(fixture.componentInstance.passwordMismatch).toBe(true)
  })

  it('submits the token, chosen username and new password, then redirects to login', () => {
    const { fixture, httpMock } = render('raw-token')
    const router = TestBed.inject(Router)
    vi.spyOn(router, 'navigateByUrl').mockResolvedValue(true)
    fixture.componentInstance.form.setValue({
      username: 'florian',
      newPassword: 'new-s3cret!',
      confirmPassword: 'new-s3cret!',
    })

    fixture.componentInstance.submit()

    const req = httpMock.expectOne('/api/auth/activate')
    expect(req.request.method).toBe('POST')
    expect(req.request.body).toEqual({
      token: 'raw-token',
      username: 'florian',
      new_password: 'new-s3cret!',
    })
    req.flush(null)

    expect(router.navigateByUrl).toHaveBeenCalledWith('/login')
  })

  it('says the server is busy, not that the link is invalid, on a 503', () => {
    const { fixture, httpMock } = render()
    fixture.componentInstance.form.setValue({
      username: 'florian',
      newPassword: 'new-s3cret!',
      confirmPassword: 'new-s3cret!',
    })

    fixture.componentInstance.submit()
    httpMock
      .expectOne('/api/auth/activate')
      .flush({ error: 'busy' }, { status: 503, statusText: 'Service Unavailable' })
    fixture.detectChanges()

    const text = fixture.nativeElement.textContent as string
    expect(text).toContain('Service momentanément occupé')
    expect(text).not.toContain('invalide ou a expiré')
  })

  it('shows an error message when activation fails', () => {
    const { fixture, httpMock } = render()
    fixture.componentInstance.form.setValue({
      username: 'florian',
      newPassword: 'new-s3cret!',
      confirmPassword: 'new-s3cret!',
    })

    fixture.componentInstance.submit()

    httpMock.expectOne('/api/auth/activate').flush(null, { status: 400, statusText: 'Bad Request' })
    fixture.detectChanges()

    expect(fixture.nativeElement.textContent).toContain(
      "Ce lien d'activation est invalide ou a expiré",
    )
  })

  it('says the username is taken, and keeps the form, when the server refuses it', () => {
    const { fixture, httpMock } = render()
    fixture.componentInstance.form.setValue({
      username: 'florian',
      newPassword: 'new-s3cret!',
      confirmPassword: 'new-s3cret!',
    })

    fixture.componentInstance.submit()
    httpMock
      .expectOne('/api/auth/activate')
      .flush(
        { error: 'username already in use', code: 'username_taken' },
        { status: 409, statusText: 'Conflict' },
      )
    fixture.detectChanges()

    expect(fixture.nativeElement.textContent).toContain("Ce nom d'utilisateur est déjà pris.")
    expect(fixture.nativeElement.textContent).not.toContain('invalide ou a expiré')
    expect(fixture.componentInstance.form.getRawValue().username).toBe('florian')
    expect(fixture.componentInstance.submitting()).toBe(false)
  })

  it('does not submit without a username', () => {
    const { fixture, httpMock } = render()
    fixture.componentInstance.form.setValue({
      username: '',
      newPassword: 'new-s3cret!',
      confirmPassword: 'new-s3cret!',
    })

    fixture.componentInstance.submit()

    httpMock.expectNone('/api/auth/activate')
  })
})
