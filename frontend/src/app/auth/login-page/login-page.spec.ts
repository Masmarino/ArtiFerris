import { TestBed } from '@angular/core/testing'
import { By } from '@angular/platform-browser'
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing'
import { provideHttpClient } from '@angular/common/http'
import { ActivatedRoute, Router, convertToParamMap, provideRouter } from '@angular/router'
import { LoginPage } from './login-page'
import { MfaEnrollmentPage } from '../mfa-enrollment/mfa-enrollment'
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

  // ngOnInit fires GET /api/auth/sso/config on every component creation — flush it with
  // `{ type: null }` (local login) unless a test wants to exercise the LDAP or OIDC path.
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

    // The SSO config endpoint must not be hit — the fragment-token path returns early.
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

    const link: HTMLAnchorElement = fixture.nativeElement.querySelector(
      'a[href="/api/auth/sso/oidc/login"]',
    )
    link.addEventListener('click', (event) => event.preventDefault())
    link.click()

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
      fixture.componentInstance.form.setValue({ username: 'florian', password: 's3cret!' })
      fixture.componentInstance.submit()
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
      fixture.componentInstance.startSso()

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

  it('shows the "Créer un compte" link when registration is enabled', () => {
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

  it('links to the public explorer', () => {
    const fixture = TestBed.createComponent(LoginPage)
    fixture.detectChanges()
    flushSsoConfig()
    fixture.detectChanges()

    const link: HTMLAnchorElement = fixture.nativeElement.querySelector('a[href="/explorer"]')
    expect(link.textContent).toContain('Explorer les paquets publics')
  })

  it('does not send a second login request when submit is called again while one is in flight', () => {
    const fixture = TestBed.createComponent(LoginPage)
    const component = fixture.componentInstance
    fixture.detectChanges()
    flushSsoConfig()
    component.form.setValue({ username: 'florian', password: 's3cret!' })

    component.submit()
    expect(component.submitting()).toBe(true)

    // A second submit attempt while the first request is still pending must be a no-op.
    component.submit()

    // If the guard didn't work, a second matching request would exist here and
    // httpMock.expectOne would throw "matches multiple requests".
    const req = httpMock.expectOne('/api/auth/login')
    expect(req.request.method).toBe('POST')
  })

  it('posts to the LDAP endpoint when the organization uses LDAP', () => {
    const fixture = TestBed.createComponent(LoginPage)
    const component = fixture.componentInstance
    fixture.detectChanges()
    flushSsoConfig('ldap')
    component.form.setValue({ username: 'florian', password: 's3cret!' })

    component.submit()

    const req = httpMock.expectOne('/api/auth/sso/ldap')
    expect(req.request.method).toBe('POST')
    expect(req.request.body).toEqual({ username: 'florian', password: 's3cret!' })
    req.flush({
      token: 'a-jwt-token',
      mfa_token: null,
      mfa_setup_required: false,
      mfa_has_totp: false,
      mfa_has_passkey: false,
    })
  })

  it('shows the OIDC redirect link when the organization uses OIDC', () => {
    const fixture = TestBed.createComponent(LoginPage)
    fixture.detectChanges()
    flushSsoConfig('oidc')
    fixture.detectChanges()

    const link: HTMLAnchorElement = fixture.nativeElement.querySelector(
      'a[href="/api/auth/sso/oidc/login"]',
    )
    expect(link).toBeTruthy()
    expect(fixture.nativeElement.querySelector('form')).toBeNull()
  })

  it('switches to the MFA code form when the login response requires a second factor', () => {
    const fixture = TestBed.createComponent(LoginPage)
    const component = fixture.componentInstance
    fixture.detectChanges()
    flushSsoConfig()
    component.form.setValue({ username: 'florian', password: 's3cret!' })

    component.submit()

    httpMock.expectOne('/api/auth/login').flush({
      token: null,
      mfa_token: 'pending-token',
      mfa_setup_required: false,
      mfa_has_totp: true,
      mfa_has_passkey: false,
    })
    fixture.detectChanges()

    expect(component.mfaToken()).toBe('pending-token')
  })

  // Regression test: an account with only a passkey (no TOTP ever confirmed) used to still
  // see a "enter your code" field on this screen — meaningless and confusing, since no such
  // code exists. The verify step must show only the factor(s) the account actually has.
  it('shows only the passkey button, not the code form, when the account has no TOTP', () => {
    // passkeysSupported() is read once at construction — jsdom has no WebAuthn API by
    // default, so this must be in place before the component is created.
    Object.defineProperty(navigator, 'credentials', {
      configurable: true,
      value: { create: () => Promise.resolve() },
    })
    const fixture = TestBed.createComponent(LoginPage)
    fixture.detectChanges()
    flushSsoConfig()
    fixture.componentInstance.form.setValue({ username: 'florian', password: 's3cret!' })
    fixture.componentInstance.submit()
    httpMock.expectOne('/api/auth/login').flush({
      token: null,
      mfa_token: 'pending-token',
      mfa_setup_required: false,
      mfa_has_totp: false,
      mfa_has_passkey: true,
    })
    fixture.detectChanges()

    expect(fixture.nativeElement.textContent).not.toContain('Code de vérification')
    expect(fixture.nativeElement.textContent).toContain("Utiliser une clé d'accès")
  })

  it('shows only the code form, not the passkey button, when the account has no passkey', () => {
    const fixture = TestBed.createComponent(LoginPage)
    fixture.detectChanges()
    flushSsoConfig()
    fixture.componentInstance.form.setValue({ username: 'florian', password: 's3cret!' })
    fixture.componentInstance.submit()
    httpMock.expectOne('/api/auth/login').flush({
      token: null,
      mfa_token: 'pending-token',
      mfa_setup_required: false,
      mfa_has_totp: true,
      mfa_has_passkey: false,
    })
    fixture.detectChanges()

    expect(fixture.nativeElement.textContent).toContain('Code de vérification')
    expect(fixture.nativeElement.textContent).not.toContain("Utiliser une clé d'accès")
  })

  it('submits the TOTP code and redirects on success', () => {
    const fixture = TestBed.createComponent(LoginPage)
    const component = fixture.componentInstance
    const router = TestBed.inject(Router)
    vi.spyOn(router, 'navigateByUrl').mockResolvedValue(true)
    fixture.detectChanges()
    flushSsoConfig()
    component.form.setValue({ username: 'florian', password: 's3cret!' })
    component.submit()
    httpMock.expectOne('/api/auth/login').flush({
      token: null,
      mfa_token: 'pending-token',
      mfa_setup_required: false,
      mfa_has_totp: true,
      mfa_has_passkey: false,
    })
    fixture.detectChanges()

    component.mfaForm.setValue({ code: '123456' })
    component.submitMfa()

    const req = httpMock.expectOne('/api/auth/mfa/verify')
    expect(req.request.body).toEqual({
      mfa_token: 'pending-token',
      code: '123456',
      backup_code: undefined,
    })
    req.flush({
      token: 'a-jwt-token',
      mfa_token: null,
      mfa_setup_required: false,
      mfa_has_totp: false,
      mfa_has_passkey: false,
    })

    expect(router.navigateByUrl).toHaveBeenCalledWith('/')
  })

  it('submits a backup code instead of a TOTP code once toggled', () => {
    const fixture = TestBed.createComponent(LoginPage)
    const component = fixture.componentInstance
    fixture.detectChanges()
    flushSsoConfig()
    component.form.setValue({ username: 'florian', password: 's3cret!' })
    component.submit()
    httpMock.expectOne('/api/auth/login').flush({
      token: null,
      mfa_token: 'pending-token',
      mfa_setup_required: false,
      mfa_has_totp: true,
      mfa_has_passkey: false,
    })
    fixture.detectChanges()

    component.toggleBackupCode()
    component.mfaForm.setValue({ code: 'abc123' })
    component.submitMfa()

    const req = httpMock.expectOne('/api/auth/mfa/verify')
    expect(req.request.body).toEqual({
      mfa_token: 'pending-token',
      code: undefined,
      backup_code: 'abc123',
    })
    req.flush({
      token: 'a-jwt-token',
      mfa_token: null,
      mfa_setup_required: false,
      mfa_has_totp: false,
      mfa_has_passkey: false,
    })
  })

  it('shows an error message when the mfa code is rejected', () => {
    const fixture = TestBed.createComponent(LoginPage)
    const component = fixture.componentInstance
    fixture.detectChanges()
    flushSsoConfig()
    component.form.setValue({ username: 'florian', password: 's3cret!' })
    component.submit()
    httpMock.expectOne('/api/auth/login').flush({
      token: null,
      mfa_token: 'pending-token',
      mfa_setup_required: false,
      mfa_has_totp: true,
      mfa_has_passkey: false,
    })
    fixture.detectChanges()

    component.mfaForm.setValue({ code: '000000' })
    component.submitMfa()
    httpMock
      .expectOne('/api/auth/mfa/verify')
      .flush(null, { status: 401, statusText: 'Unauthorized' })
    fixture.detectChanges()

    expect(fixture.nativeElement.textContent).toContain('Code invalide.')
  })

  it.each([
    [503, 'Service momentanément occupé, réessayez'],
    [429, 'Trop de tentatives, réessayez plus tard.'],
    [401, 'Identifiants invalides'],
  ])('shows the right message when the login answers a %i', (status, message) => {
    const fixture = TestBed.createComponent(LoginPage)
    const component = fixture.componentInstance
    fixture.detectChanges()
    flushSsoConfig()
    component.form.setValue({ username: 'florian', password: 's3cret!' })

    component.submit()
    httpMock.expectOne('/api/auth/login').flush({ error: 'x' }, { status, statusText: 'x' })
    fixture.detectChanges()

    expect(fixture.nativeElement.textContent).toContain(message)
    expect(component.submitting()).toBe(false)
  })

  it('logs in with a passkey when the second factor is a passkey', async () => {
    Object.defineProperty(navigator, 'credentials', {
      configurable: true,
      value: {
        get: () =>
          Promise.resolve({
            id: 'cred-1',
            type: 'public-key',
            rawId: new Uint8Array([1, 2, 3]).buffer,
            response: {
              authenticatorData: new Uint8Array([4, 5, 6]).buffer,
              clientDataJSON: new Uint8Array([7, 8, 9]).buffer,
              signature: new Uint8Array([10, 11, 12]).buffer,
              userHandle: null,
            },
          }),
      },
    })
    const fixture = TestBed.createComponent(LoginPage)
    const component = fixture.componentInstance
    const router = TestBed.inject(Router)
    vi.spyOn(router, 'navigateByUrl').mockResolvedValue(true)
    fixture.detectChanges()
    flushSsoConfig()
    component.form.setValue({ username: 'florian', password: 's3cret!' })
    component.submit()
    httpMock.expectOne('/api/auth/login').flush({
      token: null,
      mfa_token: 'pending-token',
      mfa_setup_required: false,
      mfa_has_totp: false,
      mfa_has_passkey: true,
    })
    fixture.detectChanges()

    const loginPromise = component.submitPasskey()

    const startReq = httpMock.expectOne('/api/auth/mfa/passkey/start')
    expect(startReq.request.body).toEqual({ mfa_token: 'pending-token' })
    startReq.flush({
      challenge_id: 'challenge-1',
      public_key: {
        challenge: 'AQID',
        rpId: 'x',
        allowCredentials: [],
        userVerification: 'required',
      },
    })
    await new Promise((resolve) => setTimeout(resolve, 0))

    const finishReq = httpMock.expectOne('/api/auth/mfa/passkey/finish')
    expect(finishReq.request.body.mfa_token).toBe('pending-token')
    expect(finishReq.request.body.challenge_id).toBe('challenge-1')
    finishReq.flush({
      token: 'a-jwt-token',
      mfa_token: null,
      mfa_setup_required: false,
      mfa_has_totp: false,
      mfa_has_passkey: false,
    })
    await loginPromise

    expect(router.navigateByUrl).toHaveBeenCalledWith('/')

    Object.defineProperty(navigator, 'credentials', { configurable: true, value: undefined })
  })

  it('renders app-mfa-enrollment with the pending mfa token when the account has no second factor enrolled yet', () => {
    const fixture = TestBed.createComponent(LoginPage)
    const component = fixture.componentInstance
    fixture.detectChanges()
    flushSsoConfig()
    component.form.setValue({ username: 'florian', password: 's3cret!' })

    component.submit()
    httpMock.expectOne('/api/auth/login').flush({
      token: null,
      mfa_token: 'pending-token',
      mfa_setup_required: true,
      mfa_has_totp: false,
      mfa_has_passkey: false,
    })
    fixture.detectChanges()

    expect(component.mfaSetupRequired()).toBe(true)
    const enrollment = fixture.debugElement.query(By.directive(MfaEnrollmentPage))
    expect(enrollment).toBeTruthy()
    expect((enrollment.componentInstance as MfaEnrollmentPage).mfaToken()).toBe('pending-token')
  })

  it('redirects home when the extracted enrollment component reports completion', () => {
    const fixture = TestBed.createComponent(LoginPage)
    const component = fixture.componentInstance
    const router = TestBed.inject(Router)
    vi.spyOn(router, 'navigateByUrl').mockResolvedValue(true)
    fixture.detectChanges()
    flushSsoConfig()

    component.onEnrollmentCompleted()

    expect(router.navigateByUrl).toHaveBeenCalledWith('/')
  })
})
