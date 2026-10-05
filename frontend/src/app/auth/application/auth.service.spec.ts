import { TestBed } from '@angular/core/testing'
import { AuthService } from './auth.service'
import { PageTitleService } from '../../shell/page-title.service'
import { ToastService } from '../../shared/toast.service'
import { AUTH_PORT, AuthPort } from './auth.port'
import { SessionToken } from './session-token'

describe('AuthService', () => {
  function setup(port: Partial<AuthPort>) {
    TestBed.configureTestingModule({
      providers: [AuthService, { provide: AUTH_PORT, useValue: port }],
    })
    return TestBed.inject(AuthService)
  }

  afterEach(() => {
    sessionStorage.clear()
  })

  it('clears the token on logout', () => {
    const service = setup({})
    TestBed.inject(SessionToken).set('a-jwt-token')

    service.logout()

    expect(service.token()).toBeNull()
    expect(service.isAuthenticated()).toBe(false)
  })

  it('stores a token obtained externally after an SSO login was started', () => {
    const service = setup({})
    service.beginSsoLogin('/users/1')

    const result = service.completeExternalLogin('a-jwt-token')

    expect(result).toEqual({ returnUrl: '/users/1' })
    expect(service.token()).toBe('a-jwt-token')
    expect(service.isAuthenticated()).toBe(true)
  })

  it('refuses an external token when no SSO login was started', () => {
    const service = setup({})

    expect(service.completeExternalLogin('attacker-jwt')).toBeNull()
    expect(service.token()).toBeNull()
  })

  it('refuses an external token when the start flag has expired', () => {
    const service = setup({})
    sessionStorage.setItem(
      'artiferris_sso_pending',
      JSON.stringify({ startedAt: Date.now() - 11 * 60 * 1000, returnUrl: null }),
    )

    expect(service.completeExternalLogin('late-jwt')).toBeNull()
    expect(service.token()).toBeNull()
  })

  it('consumes the start flag, so a second token is refused', () => {
    const service = setup({})
    service.beginSsoLogin(null)

    expect(service.completeExternalLogin('first')).not.toBeNull()
    service.logout()
    expect(service.completeExternalLogin('second')).toBeNull()
    expect(service.token()).toBeNull()
  })

  it('drops an unsafe return URL that was stored with the flag', () => {
    const service = setup({})
    sessionStorage.setItem(
      'artiferris_sso_pending',
      JSON.stringify({ startedAt: Date.now(), returnUrl: '//evil.example' }),
    )

    expect(service.completeExternalLogin('jwt')).toEqual({ returnUrl: null })
  })

  it('fails closed when sessionStorage throws', () => {
    const service = setup({})
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('denied')
    })

    expect(service.completeExternalLogin('jwt')).toBeNull()
    vi.restoreAllMocks()
  })

  it('clears the page title and the queued toasts on logout', () => {
    const service = setup({})
    const pageTitle = TestBed.inject(PageTitleService)
    const toasts = TestBed.inject(ToastService)
    pageTitle.title.set('acme-private-repo')
    toasts.info('Dépôt privé créé')

    service.logout()

    expect(pageTitle.title()).toBe('')
    expect(toasts.toasts()).toEqual([])
  })
})
