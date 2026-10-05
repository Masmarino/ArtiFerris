import { TestBed } from '@angular/core/testing'
import { By } from '@angular/platform-browser'
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing'
import { provideHttpClient } from '@angular/common/http'
import { ActivatedRoute, convertToParamMap, provideRouter } from '@angular/router'
import { AuthResetPassword } from '@masmarino/gabarit/auth-reset-password'
import { ResetPasswordPage } from './reset-password-page'
import { authProviders } from '../infrastructure/auth.providers'

const TOKEN = 'ab12'.repeat(16)

function render(fragment: string | null = `token=${TOKEN}`) {
  TestBed.configureTestingModule({
    imports: [ResetPasswordPage],
    providers: [
      provideHttpClient(),
      provideHttpClientTesting(),
      provideRouter([]),
      ...authProviders,
      {
        provide: ActivatedRoute,
        useValue: { snapshot: { fragment, queryParamMap: convertToParamMap({}) } },
      },
    ],
  })
  const fixture = TestBed.createComponent(ResetPasswordPage)
  const httpMock = TestBed.inject(HttpTestingController)
  fixture.detectChanges()
  const form = fixture.debugElement.query(By.directive(AuthResetPassword))
    .componentInstance as AuthResetPassword
  return { fixture, httpMock, form, el: fixture.nativeElement as HTMLElement }
}

function fill(form: AuthResetPassword): void {
  form['password'].set('n3w-s3cret!')
  form['confirmation'].set('n3w-s3cret!')
}

describe('ResetPasswordPage', () => {
  afterEach(() => {
    TestBed.inject(HttpTestingController).verify()
  })

  it('says why the password is to be chosen again', () => {
    const { el } = render()

    expect(el.querySelector('h1')?.textContent).toContain('Choisissez un nouveau mot de passe')
    expect(el.textContent).toContain('votre ancien mot de passe ne fonctionne plus.')
  })

  it('sends the token and the new password, then says it is saved', () => {
    const { fixture, httpMock, form, el } = render()
    fill(form)

    form.submit()
    const req = httpMock.expectOne('/api/auth/reset-password')
    expect(req.request.body).toEqual({ token: TOKEN, new_password: 'n3w-s3cret!' })
    req.flush(null)
    fixture.detectChanges()

    expect(el.querySelector('h1')?.textContent).toContain('Votre mot de passe est enregistré')
  })

  it('shows the dead link, without any request, when the link has no token', () => {
    const { el } = render(null)

    expect(el.querySelector('h1')?.textContent).toContain('Ce lien ne fonctionne pas')
    expect(el.querySelector('form')).toBeNull()
  })

  it('turns into the dead link when the server refuses the link', () => {
    const { fixture, httpMock, form, el } = render()
    fill(form)

    form.submit()
    httpMock
      .expectOne('/api/auth/reset-password')
      .flush(
        { error: 'invalid or expired password reset link', code: 'password_reset_link_invalid' },
        { status: 400, statusText: 'Bad Request' },
      )
    fixture.detectChanges()

    expect(el.querySelector('h1')?.textContent).toContain('Ce lien ne fonctionne pas')
    expect(el.textContent).toContain('Demandez à un administrateur de recommencer.')
  })
})
