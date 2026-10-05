import { TestBed } from '@angular/core/testing'
import { firstValueFrom, of } from 'rxjs'
import { AuthPort } from '../application/auth.port'
import { SessionToken } from '../application/session-token'
import { KitAuthAdapter } from './kit-auth.adapter'
import { BACKUP_CODES, fakeAuthPort, mfaPending, session, withAuthPort } from './auth-story-helpers'

describe('KitAuthAdapter', () => {
  function setup(overrides: Partial<AuthPort> = {}) {
    const port = fakeAuthPort(overrides)
    TestBed.configureTestingModule({ providers: [withAuthPort(port)] })
    return {
      adapter: TestBed.inject(KitAuthAdapter),
      session: TestBed.inject(SessionToken),
      port,
    }
  }

  afterEach(() => sessionStorage.clear())

  it('reads whether registration is open from the organisation settings', async () => {
    const { adapter } = setup({
      getSsoConfig: () => of({ type: null, registration_enabled: false }),
    })

    expect(await firstValueFrom(adapter.authConfig())).toEqual({
      registrationEnabled: false,
      passkeysAvailable: true,
    })
  })

  it('signs in against the directory once the organisation says it uses LDAP', async () => {
    const login = vi.fn(() => of(session()))
    const loginWithLdap = vi.fn(() => of(session()))
    const { adapter } = setup({
      getSsoConfig: () => of({ type: 'ldap' as const, registration_enabled: false }),
      login,
      loginWithLdap,
    })
    await firstValueFrom(adapter.authConfig())

    await firstValueFrom(adapter.login('florian', 's3cret!'))

    expect(loginWithLdap).toHaveBeenCalledWith('florian', 's3cret!')
    expect(login).not.toHaveBeenCalled()
  })

  it('stores the session of a completed sign-in, and none while a second factor is pending', async () => {
    const { adapter, session: token } = setup({ login: () => of(mfaPending()) })

    const pending = await firstValueFrom(adapter.login('florian', 's3cret!'))

    expect(pending).toEqual({
      token: null,
      mfaToken: 'mfa-token',
      mfaSetupRequired: false,
      mfaHasTotp: true,
      mfaHasPasskey: false,
    })
    expect(token.value()).toBeNull()

    await firstValueFrom(adapter.verifyMfa('mfa-token', { backupCode: 'abcd' }))

    expect(token.value()).toBe('session-jwt')
  })

  it('passes the invitee’s username along with the password', async () => {
    const activate = vi.fn(() => of(undefined))
    const { adapter } = setup({ activate })

    await firstValueFrom(adapter.activate('t0k3n', 'sup3r-s3cret!', 'marie'))

    expect(activate).toHaveBeenCalledWith('t0k3n', 'marie', 'sup3r-s3cret!')
  })

  it('does not store the session of a first enrolment: the kit does, once the codes are saved', async () => {
    const { adapter, session: token } = setup({
      finishPasskeySetup: () => of({ token: 'session-jwt', backup_codes: BACKUP_CODES }),
    })

    const totp = await firstValueFrom(adapter.confirmTotp('mfa-token', '123456'))
    const passkey = await firstValueFrom(
      adapter.finishPasskeySetup('mfa-token', 'challenge', {}, 'MacBook'),
    )

    expect(totp.backupCodes).toHaveLength(10)
    expect(passkey).toEqual({ token: 'session-jwt', backupCodes: BACKUP_CODES })
    expect(token.value()).toBeNull()
  })
})
