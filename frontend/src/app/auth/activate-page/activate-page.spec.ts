import { TestBed } from '@angular/core/testing'
import { By } from '@angular/platform-browser'
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing'
import { provideHttpClient } from '@angular/common/http'
import { ActivatedRoute, convertToParamMap, provideRouter } from '@angular/router'
import { AuthActivate } from '@masmarino/gabarit/auth-activate'
import { ActivatePage } from './activate-page'
import { authProviders } from '../infrastructure/auth.providers'

const TOKEN = 'ab12'.repeat(16)

function render(
  link: { fragment?: string | null; token?: string } = { fragment: `token=${TOKEN}` },
) {
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
          snapshot: {
            fragment: link.fragment ?? null,
            queryParamMap: convertToParamMap(link.token ? { token: link.token } : {}),
          },
        },
      },
    ],
  })
  const fixture = TestBed.createComponent(ActivatePage)
  const httpMock = TestBed.inject(HttpTestingController)
  fixture.detectChanges()
  const form = fixture.debugElement.query(By.directive(AuthActivate))
    .componentInstance as AuthActivate
  return { fixture, httpMock, form, el: fixture.nativeElement as HTMLElement }
}

function fill(form: AuthActivate, username = 'Marie'): void {
  form['username'].set(username)
  form['password'].set('sup3r-s3cret!')
  form['confirmation'].set('sup3r-s3cret!')
}

describe('ActivatePage', () => {
  afterEach(() => {
    TestBed.inject(HttpTestingController).verify()
  })

  it('asks for a username first, the administrator having invited by e-mail only', () => {
    const { el } = render()
    const fields = Array.from(el.querySelectorAll('input')).map((input) =>
      input.id.split('-').pop(),
    )

    expect(fields).toEqual(['username', 'password', 'confirmation'])
    expect(el.textContent).toContain("Choisissez votre nom d'utilisateur et votre mot de passe.")
  })

  it('shows the dead link, without any request, when the link has no token', () => {
    const { el } = render({ fragment: null })

    expect(el.querySelector('h1')?.textContent).toContain('Ce lien ne fonctionne pas')
    expect(el.querySelector('form')).toBeNull()
  })

  it('still reads the token of an older ?token= link', () => {
    const { httpMock, form } = render({ token: TOKEN })
    fill(form)

    form.submit()

    expect(httpMock.expectOne('/api/auth/activate').request.body.token).toBe(TOKEN)
  })

  it('sends the token, the chosen username and the password, then says the account is active', () => {
    const { fixture, httpMock, form, el } = render()
    fill(form)

    form.submit()
    const req = httpMock.expectOne('/api/auth/activate')
    expect(req.request.body).toEqual({
      token: TOKEN,
      username: 'Marie',
      new_password: 'sup3r-s3cret!',
    })
    req.flush(null)
    fixture.detectChanges()

    expect(el.querySelector('h1')?.textContent).toContain('Votre compte est activé')
  })

  it('keeps the form and says so when the username is already taken', () => {
    const { fixture, httpMock, form, el } = render()
    fill(form)

    form.submit()
    httpMock
      .expectOne('/api/auth/activate')
      .flush(
        { error: 'username already taken', code: 'username_taken' },
        { status: 400, statusText: 'Bad Request' },
      )
    fixture.detectChanges()

    expect(el.querySelector('h1')?.textContent).toContain('Activez votre compte')
    expect(el.textContent).toContain("Ce nom d'utilisateur est déjà utilisé")
  })

  it('turns into the dead link when the invitation has expired', () => {
    const { fixture, httpMock, form, el } = render()
    fill(form)

    form.submit()
    httpMock
      .expectOne('/api/auth/activate')
      .flush(
        { error: 'invitation expired', code: 'invitation_expired' },
        { status: 400, statusText: 'Bad Request' },
      )
    fixture.detectChanges()

    expect(el.querySelector('h1')?.textContent).toContain('Ce lien ne fonctionne pas')
  })
})
