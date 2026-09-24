import { TestBed } from '@angular/core/testing'
import { of } from 'rxjs'
import { AuthService } from './auth.service'
import { PageTitleService } from '../../shell/page-title.service'
import { ToastService } from '../../shared/toast.service'
import { AUTH_PORT, AuthPort } from './auth.port'
import { LoginResponse } from '../domain/auth.types'

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

  it('stores the token and flips isAuthenticated on a login without mfa', () => {
    const service = setup({
      login: () =>
        of({
          token: 'a-jwt-token',
          mfa_token: null,
          mfa_setup_required: false,
          mfa_has_totp: false,
          mfa_has_passkey: false,
        }),
    })

    expect(service.isAuthenticated()).toBe(false)

    let outcome
    service.login('florian', 's3cret!').subscribe((o) => (outcome = o))

    expect(service.token()).toBe('a-jwt-token')
    expect(service.isAuthenticated()).toBe(true)
    expect(outcome).toEqual({ mfaRequired: false })
  })

  it('does not store a token and reports mfaRequired when the account has a confirmed second factor', () => {
    const service = setup({
      login: () =>
        of({
          token: null,
          mfa_token: 'pending-token',
          mfa_setup_required: false,
          mfa_has_totp: true,
          mfa_has_passkey: false,
        }),
    })

    let outcome
    service.login('florian', 's3cret!').subscribe((o) => (outcome = o))

    expect(service.token()).toBeNull()
    expect(service.isAuthenticated()).toBe(false)
    expect(outcome).toEqual({
      mfaRequired: true,
      mfaToken: 'pending-token',
      mfaSetupRequired: false,
      mfaHasTotp: true,
      mfaHasPasskey: false,
    })
  })

  it('stores the session token returned by verifyMfa', () => {
    const service = setup({
      verifyMfa: () =>
        of({
          token: 'a-jwt-token',
          mfa_token: null,
          mfa_setup_required: false,
          mfa_has_totp: false,
          mfa_has_passkey: false,
        }),
    })

    service.verifyMfa('pending-token', '123456').subscribe()

    expect(service.token()).toBe('a-jwt-token')
    expect(service.isAuthenticated()).toBe(true)
  })

  it('clears the token on logout', () => {
    const service = setup({
      login: () =>
        of({
          token: 'a-jwt-token',
          mfa_token: null,
          mfa_setup_required: false,
          mfa_has_totp: false,
          mfa_has_passkey: false,
        }),
    })
    service.login('florian', 's3cret!').subscribe()

    service.logout()

    expect(service.token()).toBeNull()
    expect(service.isAuthenticated()).toBe(false)
  })

  it('reports mfa setup required when registration returns no token', () => {
    const response: LoginResponse = {
      token: null,
      mfa_token: 'mfa-token-123',
      mfa_setup_required: true,
      mfa_has_totp: false,
      mfa_has_passkey: false,
    }
    const service = setup({ register: () => of(response) })

    let outcome
    service
      .register('florian', 'florian@example.com', 'sup3r-s3cret!')
      .subscribe((o) => (outcome = o))

    expect(outcome).toEqual({
      mfaRequired: true,
      mfaToken: 'mfa-token-123',
      mfaSetupRequired: true,
      mfaHasTotp: false,
      mfaHasPasskey: false,
    })
  })

  it('reports success when LDAP login returns a token directly', () => {
    const response: LoginResponse = {
      token: 'a-jwt-token',
      mfa_token: null,
      mfa_setup_required: false,
      mfa_has_totp: false,
      mfa_has_passkey: false,
    }
    const service = setup({ loginWithLdap: () => of(response) })

    let outcome
    service.loginWithLdap('florian', 's3cret!').subscribe((o) => (outcome = o))

    expect(outcome!.mfaRequired).toBe(false)
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
