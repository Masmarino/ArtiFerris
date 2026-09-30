import { TestBed } from '@angular/core/testing'
import { provideHttpClient } from '@angular/common/http'
import { provideHttpClientTesting } from '@angular/common/http/testing'
import { of, throwError } from 'rxjs'
import { MeService } from './me.service'
import { ME_PORT, MePort } from './me.port'
import { AuthService } from '../../auth/application/auth.service'
import { LanguageService } from '../../shared/i18n/language.service'
import { setActiveLanguage } from '../../shared/i18n/translator'
import { authProviders } from '../../auth/infrastructure/auth.providers'

describe('MeService', () => {
  function setup(port: Partial<MePort>) {
    TestBed.configureTestingModule({
      providers: [
        provideHttpClient(),
        provideHttpClientTesting(),
        ...authProviders,
        { provide: ME_PORT, useValue: port },
      ],
    })
    return TestBed.inject(MeService)
  }

  it('populates username, isSuperAdmin and createdAt from the port', () => {
    const service = setup({
      load: () =>
        of({
          id: 'user-1',
          username: 'florian',
          is_super_admin: true,
          is_organization_admin: false,
          organization_id: 'org-1',
          created_at: '2026-01-01T00:00:00Z',
        }),
    })

    service.load().subscribe()

    expect(service.username()).toBe('florian')
    expect(service.isSuperAdmin()).toBe(true)
    expect(service.createdAt()).toBe('2026-01-01T00:00:00Z')
  })

  it('exposes the organization fields from the response', () => {
    const service = setup({
      load: () =>
        of({
          id: 'user-1',
          username: 'florian',
          is_super_admin: false,
          is_organization_admin: true,
          organization_id: 'org-1',
          created_at: '2026-01-01T00:00:00Z',
        }),
    })

    service.load().subscribe()

    expect(service.organizationId()).toBe('org-1')
    expect(service.isOrganizationAdmin()).toBe(true)
  })

  it('delegates changePassword() to the port', () => {
    const changePassword = vi.fn().mockReturnValue(of(undefined))
    setup({ changePassword }).changePassword('old-s3cret!', 'new-s3cret!')

    expect(changePassword).toHaveBeenCalledWith('old-s3cret!', 'new-s3cret!')
  })

  it('shares one in-flight request across concurrent callers', () => {
    const load = vi.fn().mockReturnValue(
      of({
        id: 'user-1',
        username: 'florian',
        is_super_admin: false,
        is_organization_admin: false,
        organization_id: 'org-1',
        created_at: '2026-01-01T00:00:00Z',
      }),
    )
    const service = setup({ load })

    service.load().subscribe()
    service.load().subscribe()

    expect(load).toHaveBeenCalledTimes(1)
  })

  it('does not permanently cache a failed request — a later call retries', () => {
    const load = vi
      .fn()
      .mockReturnValueOnce(throwError(() => new Error('network blip')))
      .mockReturnValueOnce(
        of({
          id: 'user-1',
          username: 'florian',
          is_super_admin: false,
          is_organization_admin: false,
          organization_id: 'org-1',
          created_at: '2026-01-01T00:00:00Z',
        }),
      )
    const service = setup({ load })

    service.load().subscribe({ error: () => undefined })
    expect(service.username()).toBeNull()

    service.load().subscribe()

    expect(load).toHaveBeenCalledTimes(2)
    expect(service.username()).toBe('florian')
  })

  it('drops the cached identity when the auth token changes, so a new user in the same tab never sees the previous one', () => {
    const load = vi
      .fn()
      .mockReturnValueOnce(
        of({
          id: 'user-1',
          username: 'alice',
          is_super_admin: true,
          is_organization_admin: false,
          organization_id: 'org-1',
          created_at: '2026-01-01T00:00:00Z',
        }),
      )
      .mockReturnValueOnce(
        of({
          id: 'user-2',
          username: 'bob',
          is_super_admin: false,
          is_organization_admin: false,
          organization_id: 'org-1',
          created_at: '2026-01-02T00:00:00Z',
        }),
      )
    const service = setup({ load })
    const auth = TestBed.inject(AuthService)

    service.load().subscribe()
    expect(service.username()).toBe('alice')

    auth.token.set('a-different-users-token')
    TestBed.flushEffects()

    expect(service.username()).toBeNull()
    expect(service.isSuperAdmin()).toBe(false)

    service.load().subscribe()

    expect(load).toHaveBeenCalledTimes(2)
    expect(service.username()).toBe('bob')
  })

  describe('language', () => {
    const account = {
      id: 'user-1',
      username: 'florian',
      is_super_admin: false,
      is_organization_admin: false,
      organization_id: 'org-1',
      created_at: '2026-01-01T00:00:00Z',
    }

    afterEach(() => setActiveLanguage('fr'))

    it('applies the language saved on the account over the one the browser suggested', async () => {
      setActiveLanguage('en')
      const service = setup({ load: () => of({ ...account, language: 'de' }) })
      const use = vi.spyOn(TestBed.inject(LanguageService), 'use').mockResolvedValue()

      service.load().subscribe()

      expect(service.language()).toBe('de')
      expect(use).toHaveBeenCalledWith('de')
    })

    it('leaves the language alone when the account has not chosen one', () => {
      const service = setup({ load: () => of({ ...account, language: null }) })
      const use = vi.spyOn(TestBed.inject(LanguageService), 'use').mockResolvedValue()

      service.load().subscribe()

      expect(service.language()).toBeNull()
      expect(use).not.toHaveBeenCalled()
    })

    it('ignores a saved language the interface does not offer', () => {
      const service = setup({ load: () => of({ ...account, language: 'xx' }) })
      const use = vi.spyOn(TestBed.inject(LanguageService), 'use').mockResolvedValue()

      service.load().subscribe()

      expect(use).not.toHaveBeenCalled()
    })

    it('copes with a server that predates the setting', () => {
      const service = setup({ load: () => of(account) })

      service.load().subscribe()

      expect(service.language()).toBeNull()
    })

    it('does not switch again when the account language is already displayed', () => {
      setActiveLanguage('it')
      const service = setup({ load: () => of({ ...account, language: 'it' }) })
      const use = vi.spyOn(TestBed.inject(LanguageService), 'use').mockResolvedValue()

      service.load().subscribe()

      expect(use).not.toHaveBeenCalled()
    })

    it('remembers a saved choice, and forgets it when the session changes', () => {
      const setLanguage = vi.fn().mockReturnValue(of(undefined))
      const service = setup({ setLanguage })

      service.setLanguage('es').subscribe()

      expect(setLanguage).toHaveBeenCalledWith('es')
      expect(service.language()).toBe('es')
    })

    it('does not remember a choice the server refused', () => {
      const service = setup({ setLanguage: () => throwError(() => new Error('400')) })

      service.setLanguage('es').subscribe({ error: () => undefined })

      expect(service.language()).toBeNull()
    })
  })
})
