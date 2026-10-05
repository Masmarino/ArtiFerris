import { TestBed } from '@angular/core/testing'
import { By } from '@angular/platform-browser'
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing'
import { provideHttpClient } from '@angular/common/http'
import { Router, provideRouter } from '@angular/router'
import { AuthRegister } from '@masmarino/gabarit/auth-register'
import { RegisterPage } from './register-page'
import { authProviders } from '../infrastructure/auth.providers'

describe('RegisterPage', () => {
  function render(registrationEnabled = true) {
    TestBed.configureTestingModule({
      imports: [RegisterPage],
      providers: [
        provideHttpClient(),
        provideHttpClientTesting(),
        provideRouter([]),
        ...authProviders,
      ],
    })
    const fixture = TestBed.createComponent(RegisterPage)
    const httpMock = TestBed.inject(HttpTestingController)
    fixture.detectChanges()
    httpMock
      .expectOne('/api/auth/sso/config')
      .flush({ type: null, registration_enabled: registrationEnabled })
    fixture.detectChanges()
    const form = fixture.debugElement.query(By.directive(AuthRegister))
      .componentInstance as AuthRegister
    return { fixture, httpMock, form, el: fixture.nativeElement as HTMLElement }
  }

  function fill(form: AuthRegister): void {
    form['username'].set('marie')
    form['email'].set('marie@example.com')
    form['password'].set('sup3r-s3cret!')
  }

  afterEach(() => {
    TestBed.inject(HttpTestingController).verify()
    sessionStorage.clear()
  })

  it('shows the form, in French, on the graphite page', () => {
    const { el } = render()

    expect(el.classList).toContain('auth-layout')
    expect(el.querySelector('h1')?.textContent).toContain('Créer un compte')
    expect(el.querySelector('main.gbt-auth-panel > gbt-git-field')).not.toBeNull()
  })

  it('says registration is closed when the organization does not accept it', () => {
    const { el } = render(false)

    expect(el.querySelector('h1')?.textContent).toContain('Les inscriptions sont fermées')
    expect(el.querySelector('form')).toBeNull()
  })

  it('creates the account, then takes it through the mandatory enrolment', () => {
    const { fixture, httpMock, form, el } = render()
    fill(form)

    form.submit()
    const req = httpMock.expectOne('/api/auth/register')
    expect(req.request.body).toEqual({
      username: 'marie',
      email: 'marie@example.com',
      password: 'sup3r-s3cret!',
    })
    req.flush({
      token: null,
      mfa_token: 'pending-token',
      mfa_setup_required: true,
      mfa_has_totp: false,
      mfa_has_passkey: false,
    })
    fixture.detectChanges()

    expect(el.querySelector('gbt-mfa-enrollment')).not.toBeNull()
  })

  it('points at the username when it is already taken', () => {
    const { fixture, httpMock, form, el } = render()
    fill(form)

    form.submit()
    httpMock
      .expectOne('/api/auth/register')
      .flush(
        { error: 'username already taken', code: 'username_taken' },
        { status: 400, statusText: 'Bad Request' },
      )
    fixture.detectChanges()

    expect(el.textContent).toContain(
      "Ce nom d'utilisateur ou cette adresse e-mail est déjà utilisé",
    )
  })

  it('goes home once the new account is signed in', () => {
    const { form } = render()
    const router = TestBed.inject(Router)
    vi.spyOn(router, 'navigateByUrl').mockResolvedValue(true)

    form.registered.emit()

    expect(router.navigateByUrl).toHaveBeenCalledWith('/')
  })
})
