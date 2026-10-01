import { TestBed } from '@angular/core/testing'
import { provideRouter, Router } from '@angular/router'
import { provideHttpClient, withInterceptors, HttpClient } from '@angular/common/http'
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing'
import { authInterceptor } from './auth.interceptor'
import { AuthService } from './application/auth.service'
import { authProviders } from './infrastructure/auth.providers'

function configure() {
  TestBed.configureTestingModule({
    providers: [
      provideHttpClient(withInterceptors([authInterceptor])),
      provideHttpClientTesting(),
      provideRouter([]),
      ...authProviders,
    ],
  })
}

describe('authInterceptor', () => {
  afterEach(() => {
    vi.restoreAllMocks()
    sessionStorage.clear()
  })

  it('attaches the bearer token when one is stored', () => {
    configure()
    TestBed.inject(AuthService).token.set('a-jwt-token')
    const http = TestBed.inject(HttpClient)
    const httpMock = TestBed.inject(HttpTestingController)

    http.get('/api/repositories').subscribe()

    const req = httpMock.expectOne('/api/repositories')
    expect(req.request.headers.get('Authorization')).toBe('Bearer a-jwt-token')
    req.flush([])
  })

  it('does not attach the Authorization header to a request to an absolute, non-app URL', () => {
    configure()
    TestBed.inject(AuthService).token.set('a-jwt-token')
    const http = TestBed.inject(HttpClient)
    const httpMock = TestBed.inject(HttpTestingController)

    http.get('https://cdn.example.com/x').subscribe()

    const req = httpMock.expectOne('https://cdn.example.com/x')
    expect(req.request.headers.get('Authorization')).toBeNull()
    req.flush([])
  })

  it('still attaches the Authorization header to a normal relative app request', () => {
    configure()
    TestBed.inject(AuthService).token.set('a-jwt-token')
    const http = TestBed.inject(HttpClient)
    const httpMock = TestBed.inject(HttpTestingController)

    http.get('/api/repositories').subscribe()

    const req = httpMock.expectOne('/api/repositories')
    expect(req.request.headers.get('Authorization')).toBe('Bearer a-jwt-token')
    req.flush([])
  })

  it('clears the token and navigates to /login on a 401 from a non-login endpoint', () => {
    configure()
    const auth = TestBed.inject(AuthService)
    auth.token.set('an-expired-token')
    const router = TestBed.inject(Router)
    vi.spyOn(router, 'navigateByUrl')
    const http = TestBed.inject(HttpClient)
    const httpMock = TestBed.inject(HttpTestingController)

    let failed = false
    http.get('/api/repositories').subscribe({ error: () => (failed = true) })

    httpMock
      .expectOne('/api/repositories')
      .flush('Unauthorized', { status: 401, statusText: 'Unauthorized' })

    expect(auth.token()).toBeNull()
    expect(router.navigateByUrl).toHaveBeenCalledWith('/login')
    // The error still reaches the caller so local handling is not swallowed.
    expect(failed).toBe(true)
  })

  it('does not log out or navigate on a 401 from the login endpoint itself', () => {
    configure()
    const auth = TestBed.inject(AuthService)
    auth.token.set('a-jwt-token')
    const router = TestBed.inject(Router)
    vi.spyOn(router, 'navigateByUrl')
    const http = TestBed.inject(HttpClient)
    const httpMock = TestBed.inject(HttpTestingController)

    let failed = false
    http
      .post('/api/auth/login', { username: 'florian', password: 'wrong' })
      .subscribe({ error: () => (failed = true) })

    httpMock
      .expectOne('/api/auth/login')
      .flush('Unauthorized', { status: 401, statusText: 'Unauthorized' })

    // LoginPage handles this 401 itself: a global logout and redirect would loop onto /login.
    expect(auth.token()).toBe('a-jwt-token')
    expect(router.navigateByUrl).not.toHaveBeenCalled()
    expect(failed).toBe(true)
  })

  it('does not log out or navigate on a 401 from /api/auth/mfa/verify', () => {
    configure()
    const auth = TestBed.inject(AuthService)
    auth.token.set('a-jwt-token')
    const router = TestBed.inject(Router)
    vi.spyOn(router, 'navigateByUrl')
    const http = TestBed.inject(HttpClient)
    const httpMock = TestBed.inject(HttpTestingController)

    let failed = false
    http
      .post('/api/auth/mfa/verify', { mfa_token: 'a-token', code: '000000' })
      .subscribe({ error: () => (failed = true) })

    httpMock
      .expectOne('/api/auth/mfa/verify')
      .flush('Unauthorized', { status: 401, statusText: 'Unauthorized' })

    // MfaVerifyPage handles this 401 itself: a global logout and redirect would loop onto /login.
    expect(auth.token()).toBe('a-jwt-token')
    expect(router.navigateByUrl).not.toHaveBeenCalled()
    expect(failed).toBe(true)
  })

  it('does not log out or navigate on a 401 from /api/auth/sso/ldap', () => {
    configure()
    const auth = TestBed.inject(AuthService)
    auth.token.set('a-jwt-token')
    const router = TestBed.inject(Router)
    vi.spyOn(router, 'navigateByUrl')
    const http = TestBed.inject(HttpClient)
    const httpMock = TestBed.inject(HttpTestingController)

    let failed = false
    http
      .post('/api/auth/sso/ldap', { username: 'florian', password: 'wrong' })
      .subscribe({ error: () => (failed = true) })

    httpMock
      .expectOne('/api/auth/sso/ldap')
      .flush('Unauthorized', { status: 401, statusText: 'Unauthorized' })

    // LoginPage handles this 401 itself (failed LDAP bind): a global logout and redirect would loop
    // onto /login.
    expect(auth.token()).toBe('a-jwt-token')
    expect(router.navigateByUrl).not.toHaveBeenCalled()
    expect(failed).toBe(true)
  })

  it.each([
    '/api/auth/register',
    '/api/auth/activate',
    '/api/auth/mfa/passkey/start',
    '/api/auth/mfa/passkey/finish',
    '/api/auth/mfa/setup/totp/enroll',
    '/api/auth/mfa/setup/totp/confirm',
    '/api/auth/mfa/setup/passkey/start',
    '/api/auth/mfa/setup/passkey/finish',
  ])('does not log out or navigate on a 401 from %s', (url) => {
    configure()
    const auth = TestBed.inject(AuthService)
    auth.token.set('a-jwt-token')
    const router = TestBed.inject(Router)
    vi.spyOn(router, 'navigateByUrl')
    const http = TestBed.inject(HttpClient)
    const httpMock = TestBed.inject(HttpTestingController)

    http.post(url, { mfa_token: 'expired' }).subscribe({ error: () => undefined })
    httpMock.expectOne(url).flush('Unauthorized', { status: 401, statusText: 'Unauthorized' })

    expect(auth.token()).toBe('a-jwt-token')
    expect(router.navigateByUrl).not.toHaveBeenCalled()
  })

  it('still logs out on a 401 from the session-authenticated logout-all', () => {
    configure()
    const auth = TestBed.inject(AuthService)
    auth.token.set('an-expired-token')
    const router = TestBed.inject(Router)
    vi.spyOn(router, 'navigateByUrl')
    const http = TestBed.inject(HttpClient)
    const httpMock = TestBed.inject(HttpTestingController)

    http.post('/api/auth/logout-all', {}).subscribe({ error: () => undefined })
    httpMock
      .expectOne('/api/auth/logout-all')
      .flush('Unauthorized', { status: 401, statusText: 'Unauthorized' })

    expect(auth.token()).toBeNull()
    expect(router.navigateByUrl).toHaveBeenCalledWith('/login')
  })

  it('keeps the current page as returnUrl when a 401 ejects the user', () => {
    configure()
    const auth = TestBed.inject(AuthService)
    auth.token.set('an-expired-token')
    const router = TestBed.inject(Router)
    vi.spyOn(router, 'url', 'get').mockReturnValue('/repositories/abc?tab=versions')
    vi.spyOn(router, 'navigateByUrl')
    const http = TestBed.inject(HttpClient)
    const httpMock = TestBed.inject(HttpTestingController)

    http.get('/api/repositories').subscribe({ error: () => undefined })
    httpMock
      .expectOne('/api/repositories')
      .flush('Unauthorized', { status: 401, statusText: 'Unauthorized' })

    expect(router.navigateByUrl).toHaveBeenCalledWith(
      '/login?returnUrl=%2Frepositories%2Fabc%3Ftab%3Dversions',
    )
  })

  it('does not redirect again when a late 401 arrives while already on /login', () => {
    configure()
    const auth = TestBed.inject(AuthService)
    auth.token.set('an-expired-token')
    const router = TestBed.inject(Router)
    vi.spyOn(router, 'url', 'get').mockReturnValue('/login?returnUrl=%2Frepositories')
    vi.spyOn(router, 'navigateByUrl')
    const http = TestBed.inject(HttpClient)
    const httpMock = TestBed.inject(HttpTestingController)

    http.get('/api/repositories').subscribe({ error: () => undefined })
    httpMock
      .expectOne('/api/repositories')
      .flush('Unauthorized', { status: 401, statusText: 'Unauthorized' })

    expect(auth.token()).toBeNull()
    expect(router.navigateByUrl).not.toHaveBeenCalled()
  })

  it('does not log out or redirect on a 401 for an anonymous request (no token attached)', () => {
    configure()
    const auth = TestBed.inject(AuthService)
    const router = TestBed.inject(Router)
    vi.spyOn(router, 'navigateByUrl')
    const http = TestBed.inject(HttpClient)
    const httpMock = TestBed.inject(HttpTestingController)

    let failed = false
    http
      .get('/api/repositories/repo-1/packages/npm/left-pad/audit')
      .subscribe({ error: () => (failed = true) })

    httpMock
      .expectOne('/api/repositories/repo-1/packages/npm/left-pad/audit')
      .flush('sign in to run an audit', { status: 401, statusText: 'Unauthorized' })

    // An anonymous visitor has no session to lose: this 401 only means nothing is cached.
    expect(auth.token()).toBeNull()
    expect(router.navigateByUrl).not.toHaveBeenCalled()
    expect(failed).toBe(true)
  })

  it('leaves non-401 errors alone', () => {
    configure()
    const auth = TestBed.inject(AuthService)
    auth.token.set('a-jwt-token')
    const router = TestBed.inject(Router)
    vi.spyOn(router, 'navigateByUrl')
    const http = TestBed.inject(HttpClient)
    const httpMock = TestBed.inject(HttpTestingController)

    http.get('/api/repositories').subscribe({ error: () => undefined })

    httpMock
      .expectOne('/api/repositories')
      .flush('Forbidden', { status: 403, statusText: 'Forbidden' })

    expect(auth.token()).toBe('a-jwt-token')
    expect(router.navigateByUrl).not.toHaveBeenCalled()
  })
})
