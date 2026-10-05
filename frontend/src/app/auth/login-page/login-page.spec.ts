import { ComponentFixture, TestBed } from '@angular/core/testing'
import { By } from '@angular/platform-browser'
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing'
import { provideHttpClient } from '@angular/common/http'
import { ActivatedRoute, Router, convertToParamMap, provideRouter } from '@angular/router'
import { LoginPage } from './login-page'
import { AuthLogin } from '@masmarino/gabarit/auth-login'
import { authProviders } from '../infrastructure/auth.providers'
import { AuthService } from '../application/auth.service'

describe('LoginPage', () => {
  let httpMock: HttpTestingController

  beforeEach(() => {
    TestBed.configureTestingModule({
      imports: [LoginPage],
      providers: [
        provideHttpClient(),
        provideHttpClientTesting(),
        provideRouter([]),
        ...authProviders,
      ],
    })
    httpMock = TestBed.inject(HttpTestingController)
  })

  // Opening the page fires GET /api/auth/sso/config: flush it with { type: null } (local login)
  // unless a test wants LDAP or OIDC.
  // The page and the kit's form share this one request.
  function flushSsoConfig(type: 'ldap' | 'oidc' | null = null, registrationEnabled = true): void {
    httpMock
      .expectOne('/api/auth/sso/config')
      .flush({ type, registration_enabled: registrationEnabled })
  }

  afterEach(() => {
    sessionStorage.clear()
    window.location.hash = ''
    httpMock.verify()
    vi.restoreAllMocks()
  })

  it('explains that all sessions were closed when sent back by a session revocation', () => {
    const route = TestBed.inject(ActivatedRoute)
    route.snapshot = {
      queryParamMap: convertToParamMap({ reason: 'sessions-revoked' }),
    } as ActivatedRoute['snapshot']
    const fixture = TestBed.createComponent(LoginPage)
    fixture.detectChanges()
    flushSsoConfig()
    fixture.detectChanges()

    expect(fixture.nativeElement.querySelector('gbt-alert').textContent).toContain(
      'Vos sessions ont été fermées : reconnectez-vous.',
    )
  })

  it('shows no session notice for an unknown reason', () => {
    const route = TestBed.inject(ActivatedRoute)
    route.snapshot = {
      queryParamMap: convertToParamMap({ reason: '<b>hacked</b>' }),
    } as ActivatedRoute['snapshot']
    const fixture = TestBed.createComponent(LoginPage)
    fixture.detectChanges()
    flushSsoConfig()
    fixture.detectChanges()

    expect(fixture.nativeElement.querySelector('gbt-alert')).toBeNull()
  })

  it('creates the component', () => {
    const fixture = TestBed.createComponent(LoginPage)
    expect(fixture.componentInstance).toBeTruthy()
  })

  /** Fills and submits the kit's form, as a user would. */
  function signIn(
    fixture: ComponentFixture<LoginPage>,
    username = 'florian',
    password = 's3cret!',
  ) {
    const form = fixture.debugElement.query(By.directive(AuthLogin)).componentInstance as AuthLogin
    form.username.set(username)
    form['password'].set(password)
    form.submit()
  }

  function clickOidcLink(fixture: ComponentFixture<LoginPage>): void {
    const link: HTMLAnchorElement = fixture.nativeElement.querySelector(
      'a[href="/api/auth/sso/oidc/login"]',
    )
    link.addEventListener('click', (event) => event.preventDefault())
    link.click()
  }

  function withReturnUrl(returnUrl: string | null): void {
    const route = TestBed.inject(ActivatedRoute)
    route.snapshot = {
      queryParamMap: convertToParamMap(returnUrl ? { returnUrl } : {}),
    } as ActivatedRoute['snapshot']
  }

  it('reads an OIDC token from the URL fragment, applies it, and scrubs the fragment', () => {
    window.location.hash = '#token=abc123'
    const auth = TestBed.inject(AuthService)
    const router = TestBed.inject(Router)
    auth.beginSsoLogin(null)
    const completeExternalLoginSpy = vi.spyOn(auth, 'completeExternalLogin')
    const replaceStateSpy = vi.spyOn(history, 'replaceState')
    vi.spyOn(router, 'navigateByUrl').mockResolvedValue(true)

    const fixture = TestBed.createComponent(LoginPage)
    fixture.detectChanges()

    // The fragment-token path returns before the SSO config request.
    httpMock.expectNone('/api/auth/sso/config')

    expect(completeExternalLoginSpy).toHaveBeenCalledWith('abc123')
    expect(auth.token()).toBe('abc123')
    expect(replaceStateSpy).toHaveBeenCalledTimes(1)
    const [, , url] = replaceStateSpy.mock.calls[0]
    expect(url).not.toContain('#')
    expect(router.navigateByUrl).toHaveBeenCalledWith('/')
  })

  it('ignores a fragment token when this tab never started an SSO login, and says so', () => {
    window.location.hash = '#token=attacker-jwt'
    const auth = TestBed.inject(AuthService)
    const router = TestBed.inject(Router)
    const replaceStateSpy = vi.spyOn(history, 'replaceState')
    vi.spyOn(router, 'navigateByUrl').mockResolvedValue(true)

    const fixture = TestBed.createComponent(LoginPage)
    fixture.detectChanges()
    flushSsoConfig()
    fixture.detectChanges()

    expect(auth.token()).toBeNull()
    expect(sessionStorage.getItem('artiferris_token')).toBeNull()
    expect(router.navigateByUrl).not.toHaveBeenCalled()
    expect(replaceStateSpy).toHaveBeenCalledTimes(1)
    expect(replaceStateSpy.mock.calls[0][2]).not.toContain('#')
    expect(fixture.nativeElement.textContent).toContain('Lien de connexion invalide ou expiré.')
    expect(fixture.nativeElement.querySelector('form')).not.toBeNull()
  })

  it('ignores a fragment token once the SSO start flag has expired', () => {
    window.location.hash = '#token=late'
    sessionStorage.setItem(
      'artiferris_sso_pending',
      JSON.stringify({ startedAt: Date.now() - 11 * 60 * 1000, returnUrl: null }),
    )
    const auth = TestBed.inject(AuthService)

    const fixture = TestBed.createComponent(LoginPage)
    fixture.detectChanges()
    flushSsoConfig()

    expect(auth.token()).toBeNull()
  })

  it('accepts a fragment token only once', () => {
    const auth = TestBed.inject(AuthService)
    auth.beginSsoLogin(null)
    vi.spyOn(TestBed.inject(Router), 'navigateByUrl').mockResolvedValue(true)

    window.location.hash = '#token=first'
    TestBed.createComponent(LoginPage).detectChanges()
    auth.logout()
    window.location.hash = '#token=replayed'
    TestBed.createComponent(LoginPage).detectChanges()
    flushSsoConfig()

    expect(auth.token()).toBeNull()
  })

  it('records the SSO start right before following the OIDC link', () => {
    const fixture = TestBed.createComponent(LoginPage)
    fixture.detectChanges()
    flushSsoConfig('oidc')
    fixture.detectChanges()

    clickOidcLink(fixture)

    expect(sessionStorage.getItem('artiferris_sso_pending')).not.toBeNull()
  })

  it('forgets an abandoned SSO start when the page opens without a return trip', () => {
    const auth = TestBed.inject(AuthService)
    auth.beginSsoLogin(null)

    const fixture = TestBed.createComponent(LoginPage)
    fixture.detectChanges()
    flushSsoConfig()

    expect(sessionStorage.getItem('artiferris_sso_pending')).toBeNull()
    expect(auth.completeExternalLogin('late-token')).toBeNull()
  })

  describe('returning to the requested page after login', () => {
    function loginAs(fixture: ReturnType<typeof TestBed.createComponent<LoginPage>>): void {
      fixture.detectChanges()
      flushSsoConfig()
      signIn(fixture)
      httpMock.expectOne('/api/auth/login').flush({
        token: 'a-jwt-token',
        mfa_token: null,
        mfa_setup_required: false,
        mfa_has_totp: false,
        mfa_has_passkey: false,
      })
    }

    it('goes back to a same-origin deep link', () => {
      withReturnUrl('/repositories/repo-1?tab=packages')
      const router = TestBed.inject(Router)
      vi.spyOn(router, 'navigateByUrl').mockResolvedValue(true)

      loginAs(TestBed.createComponent(LoginPage))

      expect(router.navigateByUrl).toHaveBeenCalledWith('/repositories/repo-1?tab=packages')
    })

    it.each([
      'https://evil.example/x',
      '//evil.example/x',
      '/\\evil.example',
      'javascript:alert(1)',
      'repositories',
      '/login',
      '/\t/evil.example',
    ])('falls back to / for %j', (returnUrl) => {
      withReturnUrl(returnUrl)
      const router = TestBed.inject(Router)
      vi.spyOn(router, 'navigateByUrl').mockResolvedValue(true)

      loginAs(TestBed.createComponent(LoginPage))

      expect(router.navigateByUrl).toHaveBeenCalledWith('/')
    })

    it('carries the deep link through the SSO round trip', () => {
      withReturnUrl('/users/u-1')
      const fixture = TestBed.createComponent(LoginPage)
      fixture.detectChanges()
      flushSsoConfig('oidc')
      fixture.detectChanges()
      clickOidcLink(fixture)

      window.location.hash = '#token=abc'
      const router = TestBed.inject(Router)
      vi.spyOn(router, 'navigateByUrl').mockResolvedValue(true)
      TestBed.createComponent(LoginPage).detectChanges()

      expect(router.navigateByUrl).toHaveBeenCalledWith('/users/u-1')
    })
  })

  it('does nothing when the URL fragment has no token', () => {
    window.location.hash = ''
    const auth = TestBed.inject(AuthService)
    const router = TestBed.inject(Router)
    const completeExternalLoginSpy = vi.spyOn(auth, 'completeExternalLogin')
    const replaceStateSpy = vi.spyOn(history, 'replaceState')
    vi.spyOn(router, 'navigateByUrl').mockResolvedValue(true)

    const fixture = TestBed.createComponent(LoginPage)
    fixture.detectChanges()
    flushSsoConfig()

    expect(completeExternalLoginSpy).not.toHaveBeenCalled()
    expect(replaceStateSpy).not.toHaveBeenCalled()
    expect(router.navigateByUrl).not.toHaveBeenCalled()
  })

  it('shows the "Créer un compte" link when registration is enabled', async () => {
    const fixture = TestBed.createComponent(LoginPage)
    fixture.detectChanges()
    flushSsoConfig(null, true)
    fixture.detectChanges()

    expect(fixture.nativeElement.textContent).toContain('Créer un compte')
  })

  it('hides the "Créer un compte" link when the admin has disabled registration', () => {
    const fixture = TestBed.createComponent(LoginPage)
    fixture.detectChanges()
    flushSsoConfig(null, false)
    fixture.detectChanges()

    expect(fixture.nativeElement.textContent).not.toContain('Créer un compte')
  })

  it('links to the public explorer, outside the panel', () => {
    const fixture = TestBed.createComponent(LoginPage)
    fixture.detectChanges()
    flushSsoConfig()
    fixture.detectChanges()

    const link: HTMLAnchorElement = fixture.nativeElement.querySelector('a[href="/explorer"]')
    expect(link.textContent).toContain('Explorer les paquets publics')
    expect(link.closest('gbt-auth-login')).toBeNull()
  })

  it('shows the notices first in the form', () => {
    const route = TestBed.inject(ActivatedRoute)
    route.snapshot = {
      queryParamMap: convertToParamMap({ reason: 'sessions-revoked' }),
    } as ActivatedRoute['snapshot']
    const fixture = TestBed.createComponent(LoginPage)
    fixture.detectChanges()
    flushSsoConfig()
    fixture.detectChanges()

    const form: HTMLFormElement = fixture.nativeElement.querySelector('form')
    expect(form.firstElementChild?.matches('[auth-notice]')).toBe(true)
  })

  it('posts to the LDAP endpoint when the organization uses LDAP', () => {
    const fixture = TestBed.createComponent(LoginPage)
    fixture.detectChanges()
    flushSsoConfig('ldap')

    signIn(fixture)

    const req = httpMock.expectOne('/api/auth/sso/ldap')
    expect(req.request.body).toEqual({ username: 'florian', password: 's3cret!' })
    req.flush({
      token: 'a-jwt-token',
      mfa_token: null,
      mfa_setup_required: false,
      mfa_has_totp: false,
      mfa_has_passkey: false,
    })
  })

  it('shows the OIDC redirect link, and no form, when the organization uses OIDC', () => {
    const fixture = TestBed.createComponent(LoginPage)
    fixture.detectChanges()
    flushSsoConfig('oidc')
    fixture.detectChanges()

    const link: HTMLAnchorElement = fixture.nativeElement.querySelector(
      'a[href="/api/auth/sso/oidc/login"]',
    )
    expect(link.textContent).toContain("Se connecter avec le fournisseur d'identité")
    expect(fixture.nativeElement.querySelector('form')).toBeNull()
    expect(fixture.nativeElement.querySelector('h1').textContent).toContain('Connexion')
  })

  it('says the credentials are wrong on a 401, in French', async () => {
    const fixture = TestBed.createComponent(LoginPage)
    fixture.detectChanges()
    flushSsoConfig()

    signIn(fixture)
    httpMock
      .expectOne('/api/auth/login')
      .flush({ error: 'invalid credentials' }, { status: 401, statusText: 'Unauthorized' })
    fixture.detectChanges()

    expect(fixture.nativeElement.textContent).toContain(
      "Nom d'utilisateur ou mot de passe incorrect",
    )
  })

  it('asks for the second factor, then signs in and goes home', () => {
    const router = TestBed.inject(Router)
    vi.spyOn(router, 'navigateByUrl').mockResolvedValue(true)
    const fixture = TestBed.createComponent(LoginPage)
    fixture.detectChanges()
    flushSsoConfig()

    signIn(fixture)
    httpMock.expectOne('/api/auth/login').flush({
      token: null,
      mfa_token: 'pending-token',
      mfa_setup_required: false,
      mfa_has_totp: true,
      mfa_has_passkey: false,
    })
    fixture.detectChanges()
    expect(fixture.nativeElement.querySelector('h1').textContent).toContain(
      'Vérification en deux étapes',
    )

    const form = fixture.debugElement.query(By.directive(AuthLogin)).componentInstance as AuthLogin
    form['code'].set('123456')
    form.verify()
    const verify = httpMock.expectOne('/api/auth/mfa/verify')
    expect(verify.request.body).toEqual({
      mfa_token: 'pending-token',
      code: '123456',
      backup_code: undefined,
    })
    verify.flush({
      token: 'a-jwt-token',
      mfa_token: null,
      mfa_setup_required: false,
      mfa_has_totp: false,
      mfa_has_passkey: false,
    })

    expect(TestBed.inject(AuthService).token()).toBe('a-jwt-token')
    expect(router.navigateByUrl).toHaveBeenCalledWith('/')
  })

  it('takes an account without a second factor through the enrolment', () => {
    const fixture = TestBed.createComponent(LoginPage)
    fixture.detectChanges()
    flushSsoConfig()

    signIn(fixture)
    httpMock.expectOne('/api/auth/login').flush({
      token: null,
      mfa_token: 'pending-token',
      mfa_setup_required: true,
      mfa_has_totp: false,
      mfa_has_passkey: false,
    })
    fixture.detectChanges()

    expect(fixture.nativeElement.querySelector('gbt-mfa-enrollment')).not.toBeNull()
    expect(TestBed.inject(AuthService).token()).toBeNull()
  })
})
