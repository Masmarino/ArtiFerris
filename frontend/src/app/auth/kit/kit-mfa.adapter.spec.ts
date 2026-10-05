import { TestBed } from '@angular/core/testing'
import { provideHttpClient } from '@angular/common/http'
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing'
import type { MfaStatus, Passkey } from '@masmarino/gabarit/auth'
import { KitMfaAdapter } from './kit-mfa.adapter'

describe('KitMfaAdapter', () => {
  function setup() {
    TestBed.configureTestingModule({ providers: [provideHttpClient(), provideHttpClientTesting()] })
    return { adapter: TestBed.inject(KitMfaAdapter), http: TestBed.inject(HttpTestingController) }
  }

  afterEach(() => TestBed.inject(HttpTestingController).verify())

  it('reads the status and the passkeys in one round trip, shared by the callers that ask together', () => {
    const { adapter, http } = setup()
    const seen: MfaStatus[] = []

    adapter.status().subscribe((status) => seen.push(status))
    adapter.status().subscribe((status) => seen.push(status))
    http
      .expectOne('/api/me/mfa')
      .flush({ totp_enabled: false, backup_codes_remaining: 10, passkey_count: 1 })
    http
      .expectOne('/api/me/mfa/passkey')
      .flush([
        { id: 'pk1', name: 'MacBook', created_at: '2026-06-01T00:00:00Z', last_used_at: null },
      ])

    const expected: MfaStatus = {
      totpEnabled: false,
      backupCodesRemaining: 10,
      passkeys: [
        { id: 'pk1', name: 'MacBook', createdAt: '2026-06-01T00:00:00Z', lastUsedAt: null },
      ],
    }
    expect(seen).toEqual([expected, expected])
  })

  it('sends the current password with every change that needs it', () => {
    const { adapter, http } = setup()

    adapter.enroll('s3cret!').subscribe()
    adapter.regenerate('s3cret!').subscribe()
    adapter.disable('s3cret!').subscribe()
    adapter.startPasskeyRegistration('s3cret!').subscribe()
    adapter.deletePasskey('pk1', 's3cret!').subscribe()

    for (const url of [
      '/api/me/mfa/totp/enroll',
      '/api/me/mfa/backup-codes/regenerate',
      '/api/me/mfa/totp',
      '/api/me/mfa/passkey/register/start',
      '/api/me/mfa/passkey/pk1',
    ]) {
      expect(http.expectOne(url).request.body).toEqual({ current_password: 's3cret!' })
    }
  })

  it('answers a new passkey as the list shows it', () => {
    const { adapter, http } = setup()
    let created: Passkey | undefined

    adapter.finishPasskeyRegistration('c1', { id: 'x' }, 'YubiKey').subscribe((p) => (created = p))
    const req = http.expectOne('/api/me/mfa/passkey/register/finish')
    expect(req.request.body).toEqual({
      challenge_id: 'c1',
      credential: { id: 'x' },
      name: 'YubiKey',
    })
    req.flush({
      id: 'pk2',
      name: 'YubiKey',
      created_at: '2026-10-05T08:00:00Z',
      last_used_at: null,
    })

    expect(created).toEqual({
      id: 'pk2',
      name: 'YubiKey',
      createdAt: '2026-10-05T08:00:00Z',
      lastUsedAt: null,
    })
  })
})
