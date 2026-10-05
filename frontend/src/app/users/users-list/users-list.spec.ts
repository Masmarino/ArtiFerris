import { TestBed } from '@angular/core/testing'
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing'
import { provideHttpClient } from '@angular/common/http'
import { Router, provideRouter } from '@angular/router'
import { signal } from '@angular/core'
import { By } from '@angular/platform-browser'
import { Spinner } from '@masmarino/gabarit/spinner'
import { UsersList } from './users-list'
import { userProviders } from '../infrastructure/user.providers'
import { authProviders } from '../../auth/infrastructure/auth.providers'
import { organizationsProviders } from '../../admin/infrastructure/organizations.providers'
import { MeService } from '../../shell/application/me.service'
import { ConfirmService } from '../../shared/confirm.service'
import { AuthService } from '../../auth/application/auth.service'
import { ToastService } from '../../shared/toast.service'
import type { UserSummary } from '../domain/user.entity'
import { PageTitleService } from '../../shell/page-title.service'

const PUBLIC_ORG = { id: 'org-public', slug: 'public', display_name: 'Public', is_public: true }
const ACME_ORG = { id: 'org-acme', slug: 'acme', display_name: 'Acme Corp', is_public: false }

const now = Date.now()
const hoursAgo = (hours: number) => new Date(now - hours * 3_600_000).toISOString()

function user(overrides: Partial<UserSummary> & Pick<UserSummary, 'id' | 'username'>): UserSummary {
  return {
    is_super_admin: false,
    organization_id: 'org-public',
    email: `${overrides.username}@example.com`,
    invitation_pending: false,
    created_at: hoursAgo(24 * 30),
    invitation_expires_at: null,
    mfa_enabled: true,
    ...overrides,
  }
}

const SELF = user({ id: 'u1', username: 'florian', is_super_admin: true })
const BOB = user({ id: 'u2', username: 'bob', mfa_enabled: false, created_at: hoursAgo(24 * 3) })
const DAVE = user({
  id: 'u3',
  username: 'dave',
  invitation_pending: true,
  mfa_enabled: false,
  invitation_expires_at: hoursAgo(-19),
  created_at: hoursAgo(5),
})
const ERIN = user({
  id: 'u4',
  username: 'erin',
  invitation_pending: true,
  mfa_enabled: false,
  invitation_expires_at: hoursAgo(48),
  created_at: hoursAgo(96),
})
const ACME_USER = user({ id: 'u5', username: 'acme-user', organization_id: 'org-acme' })

describe('UsersList', () => {
  afterEach(() => {
    vi.restoreAllMocks()
  })

  function setup(options?: { isSuperAdmin?: boolean }) {
    TestBed.configureTestingModule({
      providers: [
        provideHttpClient(),
        provideHttpClientTesting(),
        provideRouter([]),
        ...userProviders,
        ...organizationsProviders,
        ...authProviders,
        {
          provide: MeService,
          useValue: {
            isSuperAdmin: signal(options?.isSuperAdmin ?? true),
            username: signal('florian'),
          },
        },
      ],
    })
    // The shell sets it from the route; the heading, its summary and its action wait for it.
    TestBed.inject(PageTitleService).title.set('Utilisateurs')
    const fixture = TestBed.createComponent(UsersList)
    const httpMock = TestBed.inject(HttpTestingController)
    return { fixture, httpMock, element: fixture.nativeElement as HTMLElement }
  }

  function load(
    fixture: ReturnType<typeof setup>['fixture'],
    httpMock: HttpTestingController,
    users: UserSummary[] = [SELF, BOB, DAVE, ERIN],
  ) {
    fixture.detectChanges()
    httpMock.expectOne('/api/users').flush(users)
    httpMock.expectOne('/api/organizations').flush([PUBLIC_ORG, ACME_ORG])
    fixture.detectChanges()
  }

  /** A reload refetches the users; the organizations may come from the service's cache. */
  function answerReload(httpMock: HttpTestingController, users: UserSummary[]) {
    httpMock.expectOne('/api/users').flush(users)
    httpMock.match('/api/organizations').forEach((request) => request.flush([PUBLIC_ORG, ACME_ORG]))
  }

  const rowOf = (element: HTMLElement, id: string) =>
    element.querySelector<HTMLElement>(`li[data-user-id="${id}"]`)

  const rowsOf = (fixture: ReturnType<typeof setup>['fixture']) =>
    fixture.componentInstance['rows']()

  it('shows a spinner while the list loads, then a row per account', () => {
    const { fixture, httpMock, element } = setup()

    fixture.detectChanges()
    expect(element.querySelector('[role="status"]')?.textContent).toContain(
      'Chargement des utilisateurs…',
    )

    load(fixture, httpMock)

    expect(element.querySelectorAll('li[data-user-id]')).toHaveLength(4)
  })

  it('sums up the accounts and the pending invitations in the header', () => {
    const { fixture, httpMock, element } = setup()

    load(fixture, httpMock)

    expect(element.querySelector('.users-list__summary')?.textContent).toContain(
      '4 comptes · 2 invitations en attente',
    )
  })

  it('counts each tab and shows only the invitations on "Invitations en attente"', () => {
    const { fixture, httpMock, element } = setup()

    load(fixture, httpMock)
    expect(element.textContent).toContain('Tous (4)')
    expect(element.textContent).toContain('Invitations en attente (2)')

    fixture.componentInstance['filter'].set('pending')
    fixture.detectChanges()

    const ids = Array.from(element.querySelectorAll<HTMLElement>('li[data-user-id]')).map(
      (row) => row.dataset['userId'],
    )
    expect(ids.sort()).toEqual(['u3', 'u4'])
  })

  it("gives each row its account state and second factor, as FerrisGit's administration does", () => {
    const { fixture, httpMock, element } = setup()

    load(fixture, httpMock)

    expect(rowOf(element, 'u1')?.textContent).toContain('Actif')
    expect(rowOf(element, 'u1')?.textContent).toContain('Double authentification active')
    expect(rowOf(element, 'u1')?.textContent).toContain('Super-administrateur')
    expect(rowOf(element, 'u2')?.textContent).toContain('Non configurée')
    expect(rowOf(element, 'u3')?.textContent).toContain('Invitation en attente')
    expect(rowOf(element, 'u3')?.textContent).toContain('expire le')
    expect(rowOf(element, 'u4')?.textContent).toContain('Invitation expirée')
    expect(rowOf(element, 'u4')?.textContent).toContain('a expiré')
    // An invitation has no second factor to speak of yet.
    expect(rowOf(element, 'u3')?.textContent).not.toContain('Non configurée')
  })

  it('marks the signed-in account and links every other one to its page', () => {
    const { fixture, httpMock, element } = setup()

    load(fixture, httpMock)

    expect(rowOf(element, 'u1')?.textContent).toContain('Vous')
    expect(rowOf(element, 'u1')?.querySelector('a')).toBeNull()
    expect(rowOf(element, 'u2')?.querySelector('a')?.getAttribute('href')).toBe('/users/u2')
  })

  it('says so when the search matches nobody', () => {
    const { fixture, httpMock, element } = setup()

    load(fixture, httpMock)
    fixture.componentInstance['search'].set('zzz')
    fixture.detectChanges()

    expect(element.textContent).toContain('Aucun utilisateur ne correspond à cette recherche')
  })

  it('shows the empty state with the invite action when there are no accounts at all', () => {
    const { fixture, httpMock, element } = setup()

    load(fixture, httpMock, [])

    expect(element.textContent).toContain('Aucun utilisateur')
    expect(
      Array.from(element.querySelectorAll('button')).filter((button) =>
        button.textContent?.includes('Inviter un utilisateur'),
      ),
    ).toHaveLength(2)
  })

  it('says the selected organization has no accounts rather than the instance', () => {
    const { fixture, httpMock, element } = setup()

    load(fixture, httpMock, [ACME_USER])

    expect(element.textContent).toContain('Aucun utilisateur dans cette organisation.')
  })

  it('shows a retryable alert when the first load fails', () => {
    const { fixture, httpMock, element } = setup()

    fixture.detectChanges()
    httpMock.expectOne('/api/users').flush('error', { status: 500, statusText: 'Server Error' })
    httpMock.expectOne('/api/organizations').flush([PUBLIC_ORG])
    fixture.detectChanges()

    expect(element.querySelector('[role="alert"]')?.textContent).toContain(
      'Impossible de charger les utilisateurs.',
    )
    expect(element.textContent).not.toContain('Chargement des utilisateurs…')
  })

  it('defaults the organization filter to the public organization, and keeps the pick across a reload', () => {
    const { fixture, httpMock, element } = setup()

    load(fixture, httpMock, [SELF, ACME_USER])
    expect(fixture.componentInstance.selectedOrganizationId()).toBe('org-public')
    expect(rowOf(element, 'u5')).toBeNull()

    fixture.componentInstance.selectedOrganizationId.set('ALL')
    fixture.componentInstance.reload()
    answerReload(httpMock, [SELF, ACME_USER])
    fixture.detectChanges()

    expect(fixture.componentInstance.selectedOrganizationId()).toBe('ALL')
    expect(rowOf(element, 'u5')?.textContent).toContain('Acme Corp')
  })

  it('offers an option per organization plus "Toutes les organisations"', () => {
    const { fixture, httpMock } = setup()

    load(fixture, httpMock)

    expect(fixture.componentInstance.organizationOptions()).toEqual([
      { value: 'ALL', label: 'Toutes les organisations' },
      { value: 'org-public', label: 'Public' },
      { value: 'org-acme', label: 'Acme Corp' },
    ])
    expect(fixture.nativeElement.querySelector('.users-list__organization')).toBeTruthy()
  })

  it('resends an invitation from the row menu and says to whom', () => {
    const { fixture, httpMock } = setup()
    const toast = vi.spyOn(TestBed.inject(ToastService), 'success')

    load(fixture, httpMock)
    fixture.componentInstance.resend(rowsOf(fixture).find((row) => row.user.id === 'u3')!)
    fixture.detectChanges()

    expect(fixture.debugElement.query(By.directive(Spinner))).toBeTruthy()
    httpMock.expectOne('/api/users/u3/resend-invitation').flush({ email_sent: true })
    answerReload(httpMock, [SELF, BOB, DAVE, ERIN])

    expect(toast).toHaveBeenCalledWith('Invitation renvoyée à dave@example.com.')
  })

  it('shows the new activation link above the list when its mail could not go out', () => {
    const { fixture, httpMock, element } = setup()
    const toast = vi.spyOn(TestBed.inject(ToastService), 'success')

    load(fixture, httpMock)
    fixture.componentInstance.resend(rowsOf(fixture).find((row) => row.user.id === 'u3')!)
    httpMock.expectOne('/api/users/u3/resend-invitation').flush({
      email_sent: false,
      email_error: 'email_send_failed',
      activation_url: 'https://app.example.com/activate#token=new',
    })
    answerReload(httpMock, [SELF, BOB, DAVE, ERIN])
    fixture.detectChanges()

    expect(toast).not.toHaveBeenCalled()
    const alert = element.querySelector('app-link-mail-failed')
    expect(alert?.textContent).toContain("Le serveur mail n'a pas accepté le message.")
    expect(alert?.textContent).toContain('Transmettez ce lien à dave@example.com')
    expect(alert?.querySelector('code')?.textContent).toBe(
      'https://app.example.com/activate#token=new',
    )
  })

  it('resets a password once confirmed and says the link went out', async () => {
    const { fixture, httpMock } = setup()
    const ask = vi.spyOn(TestBed.inject(ConfirmService), 'ask').mockResolvedValue(true)
    const toast = vi.spyOn(TestBed.inject(ToastService), 'success')

    load(fixture, httpMock)
    const bob = rowsOf(fixture).find((row) => row.user.id === 'u2')!
    expect(bob.canResetPassword).toBe(true)
    await fixture.componentInstance.resetPassword(bob)

    expect(ask).toHaveBeenCalledWith(
      expect.objectContaining({ heading: 'Réinitialiser le mot de passe' }),
    )
    httpMock.expectOne('/api/users/u2/reset-password').flush({ email_sent: true })
    expect(toast).toHaveBeenCalledWith(
      'Un lien pour choisir un nouveau mot de passe a été envoyé à bob.',
    )
  })

  it('shows the reset link above the list when its mail could not go out', async () => {
    const { fixture, httpMock, element } = setup()
    vi.spyOn(TestBed.inject(ConfirmService), 'ask').mockResolvedValue(true)

    load(fixture, httpMock)
    await fixture.componentInstance.resetPassword(
      rowsOf(fixture).find((row) => row.user.id === 'u2')!,
    )
    httpMock.expectOne('/api/users/u2/reset-password').flush({
      email_sent: false,
      email_error: 'email_no_address',
      reset_url: 'https://app.example.com/reset-password#token=abc',
    })
    fixture.detectChanges()

    const alert = element.querySelector('app-link-mail-failed')
    expect(alert?.textContent).toContain("Ce compte n'a pas d'adresse e-mail vérifiée.")
    expect(alert?.textContent).toContain('il est valable 1 heure')
    expect(alert?.querySelector('code')?.textContent).toBe(
      'https://app.example.com/reset-password#token=abc',
    )
  })

  it("offers no password reset for one's own account or an invitation", () => {
    const { fixture, httpMock } = setup()

    load(fixture, httpMock)

    expect(rowsOf(fixture).find((row) => row.isSelf)?.canResetPassword).toBe(false)
    expect(rowsOf(fixture).find((row) => row.user.id === 'u3')?.canResetPassword).toBe(false)
  })

  it('says why the server refused a password reset', async () => {
    const { fixture, httpMock } = setup()
    vi.spyOn(TestBed.inject(ConfirmService), 'ask').mockResolvedValue(true)
    const toast = vi.spyOn(TestBed.inject(ToastService), 'error')

    load(fixture, httpMock)
    await fixture.componentInstance.resetPassword(
      rowsOf(fixture).find((row) => row.user.id === 'u2')!,
    )
    httpMock
      .expectOne('/api/users/u2/reset-password')
      .flush(
        { error: 'x', code: 'password_managed_by_identity_provider' },
        { status: 400, statusText: 'Bad Request' },
      )

    expect(toast).toHaveBeenCalledWith(
      "Cette organisation se connecte par son fournisseur d'identité, qui gère ses mots de passe.",
    )
  })

  it('resets the second factors of an account once confirmed', async () => {
    const { fixture, httpMock } = setup()
    const ask = vi.spyOn(TestBed.inject(ConfirmService), 'ask').mockResolvedValue(true)
    const toast = vi.spyOn(TestBed.inject(ToastService), 'success')

    load(fixture, httpMock, [SELF, { ...BOB, mfa_enabled: true }])
    const bob = rowsOf(fixture).find((row) => row.user.id === 'u2')!
    expect(bob.canResetMfa).toBe(true)
    await fixture.componentInstance.resetMfa(bob)

    expect(ask).toHaveBeenCalledWith(
      expect.objectContaining({ message: expect.stringContaining('bob sera déconnecté') }),
    )
    const request = httpMock.expectOne('/api/users/u2/mfa')
    expect(request.request.method).toBe('DELETE')
    request.flush(null)
    expect(toast).toHaveBeenCalledWith('Double authentification réinitialisée.')
    expect(rowsOf(fixture).find((row) => row.user.id === 'u2')?.user.mfa_enabled).toBe(false)
  })

  it('offers no reset to an account without a second factor or still invited', () => {
    const { fixture, httpMock } = setup()

    load(fixture, httpMock)

    expect(rowsOf(fixture).find((row) => row.user.id === 'u2')?.canResetMfa).toBe(false)
    expect(rowsOf(fixture).find((row) => row.user.id === 'u3')?.canResetMfa).toBe(false)
  })

  it('signs the administrator out after resetting their own second factors', async () => {
    const { fixture, httpMock } = setup()
    vi.spyOn(TestBed.inject(ConfirmService), 'ask').mockResolvedValue(true)
    const logout = vi.spyOn(TestBed.inject(AuthService), 'logout')
    const navigate = vi.spyOn(TestBed.inject(Router), 'navigate').mockResolvedValue(true)

    load(fixture, httpMock)
    await fixture.componentInstance.resetMfa(rowsOf(fixture).find((row) => row.isSelf)!)
    httpMock.expectOne('/api/users/u1/mfa').flush(null)

    expect(logout).toHaveBeenCalled()
    expect(navigate).toHaveBeenCalledWith(['/login'])
  })

  it('grants super-administrator rights at once', async () => {
    const { fixture, httpMock } = setup()
    const ask = vi.spyOn(TestBed.inject(ConfirmService), 'ask')

    load(fixture, httpMock)
    await fixture.componentInstance.toggleSuperAdmin(
      rowsOf(fixture).find((row) => row.user.id === 'u2')!,
    )

    const grant = httpMock.expectOne('/api/users/u2/super-admin')
    expect(grant.request.body).toEqual({ is_super_admin: true })
    grant.flush(null)
    expect(ask).not.toHaveBeenCalled()
    expect(rowsOf(fixture).find((row) => row.user.id === 'u2')?.user.is_super_admin).toBe(true)
  })

  it('asks before removing them, and says why the last super-administrator keeps them', async () => {
    const { fixture, httpMock } = setup()
    const ask = vi.spyOn(TestBed.inject(ConfirmService), 'ask').mockResolvedValue(true)
    const toast = vi.spyOn(TestBed.inject(ToastService), 'error')

    load(fixture, httpMock)
    await fixture.componentInstance.toggleSuperAdmin(
      rowsOf(fixture).find((row) => row.user.id === 'u1')!,
    )

    expect(ask).toHaveBeenCalledWith(
      expect.objectContaining({
        message: expect.stringContaining('Vous allez vous retirer vous-même'),
      }),
    )
    httpMock
      .expectOne('/api/users/u1/super-admin')
      .flush('conflict', { status: 409, statusText: 'Conflict' })
    expect(toast).toHaveBeenCalledWith('Impossible de rétrograder le dernier super-administrateur.')
  })

  describe('as an organization admin', () => {
    it("loads only its own organization's users, with no /api/organizations call", () => {
      const { fixture, httpMock, element } = setup({ isSuperAdmin: false })

      fixture.detectChanges()
      httpMock.expectOne('/api/users').flush([ACME_USER])
      fixture.detectChanges()

      expect(rowOf(element, 'u5')).toBeTruthy()
      httpMock.verify()
    })

    it('hides the organization filter, the invite action and the super-administrator rights', () => {
      const { fixture, httpMock, element } = setup({ isSuperAdmin: false })

      fixture.detectChanges()
      httpMock.expectOne('/api/users').flush([ACME_USER, { ...DAVE, organization_id: 'org-acme' }])
      fixture.detectChanges()

      expect(element.querySelector('.users-list__organization')).toBeNull()
      expect(element.textContent).not.toContain('Inviter un utilisateur')
      expect(rowsOf(fixture).every((row) => row.adminAction === null)).toBe(true)
      expect(rowsOf(fixture).find((row) => row.user.id === 'u3')?.canResend).toBe(true)
    })

    it("can't resend the invitation of a super-administrator", () => {
      const { fixture, httpMock } = setup({ isSuperAdmin: false })

      fixture.detectChanges()
      httpMock
        .expectOne('/api/users')
        .flush([{ ...DAVE, organization_id: 'org-acme', is_super_admin: true }])
      fixture.detectChanges()

      expect(rowsOf(fixture)[0].canResend).toBe(false)
    })
  })
})
