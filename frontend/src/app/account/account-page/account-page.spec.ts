import { TestBed } from '@angular/core/testing'
import { Router, provideRouter } from '@angular/router'
import { RouterTestingHarness } from '@angular/router/testing'
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing'
import { provideHttpClient } from '@angular/common/http'
import { By } from '@angular/platform-browser'
import { formatLocalizedDate } from '../../shared/i18n/localized-date'
import { AccountPage } from './account-page'
import { MeService } from '../../shell/application/me.service'
import { SessionToken } from '../../auth/application/session-token'
import { PageTitleService } from '../../shell/page-title.service'
import { SessionSettings } from '../session-settings/session-settings'
import { ApiTokensList } from '../../tokens/api-tokens-list/api-tokens-list'
import { apiTokenProviders } from '../../tokens/infrastructure/api-token.providers'
import { MfaSettings } from '@masmarino/gabarit/mfa-settings'
import { PasskeySettings } from '@masmarino/gabarit/passkey-settings'
import { meProviders } from '../../shell/infrastructure/me.providers'
import { authProviders } from '../../auth/infrastructure/auth.providers'
import { ToastService } from '../../shared/toast.service'
import { SessionRevocationService } from '../../auth/application/session-revocation.service'

describe('AccountPage', () => {
  const signOutAndRedirect = vi.fn()

  /** The page at `/account`, on the section the address asks for (none: the profile). */
  async function render(section?: 'password' | 'security' | 'tokens') {
    signOutAndRedirect.mockClear()
    TestBed.configureTestingModule({
      providers: [
        provideHttpClient(),
        provideHttpClientTesting(),
        provideRouter([{ path: 'account', component: AccountPage }]),
        ...apiTokenProviders,
        ...meProviders,
        ...authProviders,
        { provide: SessionRevocationService, useValue: { signOutAndRedirect } },
      ],
    })
    const me = TestBed.inject(MeService)
    me.username.set('florian')
    me.isSuperAdmin.set(true)
    me.createdAt.set('2026-01-01T00:00:00Z')
    // The shell sets the route's title; here there is no shell.
    TestBed.inject(PageTitleService).title.set('Mon compte')
    const harness = await RouterTestingHarness.create()
    const page = await harness.navigateByUrl(
      section ? `/account?section=${section}` : '/account',
      AccountPage,
    )
    harness.detectChanges()
    const httpMock = TestBed.inject(HttpTestingController)
    const fixture = {
      componentInstance: page,
      nativeElement: harness.routeNativeElement as HTMLElement,
      debugElement: harness.routeDebugElement!,
      detectChanges: () => harness.detectChanges(),
    }
    return { fixture, httpMock, harness }
  }

  const navLinks = (el: HTMLElement) =>
    Array.from(el.querySelectorAll<HTMLAnchorElement>('a[gbtNavTab]'))

  it("opens on its h1, like FerrisGit's page", async () => {
    const { fixture } = await render()
    const header = fixture.nativeElement.querySelector('gbt-page-header')!

    expect(header.querySelector('h1')?.textContent?.trim()).toBe('Mon compte')
    expect(header.textContent).toContain(
      "Profil, mot de passe, double authentification et jetons d'accès",
    )
  })

  it('lists its four sections on the side, and opens on the profile', async () => {
    const { fixture } = await render()
    const links = navLinks(fixture.nativeElement)

    expect(links.map((link) => link.textContent?.trim())).toEqual([
      'Profil',
      'Mot de passe',
      'Sécurité',
      "Jetons d'accès",
    ])
    expect(links[0].getAttribute('aria-current')).toBe('page')
    expect(links[2].getAttribute('href')).toBe('/account?section=security')
  })

  it('shows the profile for a section it does not know', async () => {
    const { harness, fixture } = await render()

    await harness.navigateByUrl('/account?section=unknown')
    fixture.detectChanges()

    expect(navLinks(fixture.nativeElement)[0].getAttribute('aria-current')).toBe('page')
    expect(fixture.nativeElement.textContent).toContain("Nom d'utilisateur, non modifiable")
  })

  it('shows the username, super-admin status and member-since date', async () => {
    const { fixture } = await render()
    const text = fixture.nativeElement.textContent as string

    expect(fixture.nativeElement.querySelector('.account-page__username')?.textContent).toBe(
      'florian',
    )
    expect(text).toContain('Super-administrateur')
    const expectedDate = formatLocalizedDate('2026-01-01T00:00:00Z')
    expect(text).toContain(`Membre depuis le ${expectedDate}`)
  })

  it('shows the verified address ArtiFerris writes to, read-only', async () => {
    const { fixture } = await render()
    TestBed.inject(MeService).email.set('florian@corp.example')
    fixture.detectChanges()

    const email = fixture.nativeElement.querySelector('.account-page__email') as HTMLElement
    expect(email.querySelector('dt')?.textContent?.trim()).toBe('Email')
    expect(email.querySelector('.account-page__value')?.textContent?.trim()).toBe(
      'florian@corp.example',
    )
    expect(email.querySelector('input')).toBeNull()
  })

  it('says when there is no verified address', async () => {
    const { fixture } = await render()

    const email = fixture.nativeElement.querySelector('.account-page__email') as HTMLElement
    expect(email.querySelector('.account-page__value')?.textContent?.trim()).toBe(
      'Aucune adresse vérifiée',
    )
  })

  it('chooses the interface language from the profile, and signs out of this browser', async () => {
    const { fixture } = await render()
    const router = TestBed.inject(Router)
    const navigate = vi.spyOn(router, 'navigateByUrl').mockResolvedValue(true)

    expect(fixture.nativeElement.textContent).toContain("Langue de l'interface")
    const signOut = Array.from(
      fixture.nativeElement.querySelectorAll<HTMLButtonElement>('.account-page__session button'),
    ).find((button) => button.textContent?.includes('Déconnexion'))!
    signOut.click()

    expect(navigate).toHaveBeenCalledWith('/login')
  })

  it('changes the password and stays signed in with the fresh session, the other ones ended', async () => {
    const { fixture, httpMock } = await render('password')

    fixture.componentInstance.form.setValue({
      currentPassword: 'old-s3cret!',
      newPassword: 'new-s3cret!',
      confirmPassword: 'new-s3cret!',
    })
    fixture.componentInstance.submit()

    const req = httpMock.expectOne('/api/me/password')
    expect(req.request.body).toEqual({
      current_password: 'old-s3cret!',
      new_password: 'new-s3cret!',
    })
    req.flush({ token: 'fresh-session' })
    fixture.detectChanges()

    expect(TestBed.inject(SessionToken).value()).toBe('fresh-session')
    expect(signOutAndRedirect).not.toHaveBeenCalled()
    // Same account: the profile stays, it is not reset as for another account signing in.
    expect(TestBed.inject(MeService).username()).toBe('florian')
    expect(
      fixture.nativeElement.querySelector('.account-page__password-status')!.textContent,
    ).toContain('Mot de passe modifié.')
    expect(fixture.componentInstance.form.controls.newPassword.value).toBe('')
  })

  it('shows an error and keeps the new/confirm fields when the server rejects the change', async () => {
    const { fixture, httpMock } = await render('password')

    fixture.componentInstance.form.setValue({
      currentPassword: 'wrong',
      newPassword: 'new-s3cret!',
      confirmPassword: 'new-s3cret!',
    })
    fixture.componentInstance.submit()

    const req = httpMock.expectOne('/api/me/password')
    req.flush({ error: 'invalid credentials' }, { status: 400, statusText: 'Bad Request' })
    fixture.detectChanges()

    expect(TestBed.inject(ToastService).toasts().at(-1)).toMatchObject({
      variant: 'error',
      message: 'Impossible de modifier le mot de passe.',
    })
    expect(
      fixture.nativeElement.querySelector('.account-page__password-status')!.textContent,
    ).toContain('Mot de passe actuel incorrect.')
    expect(fixture.componentInstance.form.controls.newPassword.value).toBe('new-s3cret!')
    expect(fixture.componentInstance.form.controls.confirmPassword.value).toBe('new-s3cret!')
    expect(fixture.componentInstance.form.controls.currentPassword.value).toBe('')
  })

  it('blocks submission client-side when new and confirm passwords do not match', async () => {
    const { fixture, httpMock } = await render('password')

    fixture.componentInstance.form.setValue({
      currentPassword: 'old-s3cret!',
      newPassword: 'new-s3cret!',
      confirmPassword: 'something-else!',
    })
    expect(fixture.componentInstance.form.invalid).toBe(true)

    fixture.componentInstance.submit()

    httpMock.expectNone('/api/me/password')
  })

  it('does not show a passwords-mismatch message before the confirm field is touched', async () => {
    const { fixture } = await render('password')

    fixture.componentInstance.form.setValue({
      currentPassword: 'old-s3cret!',
      newPassword: 'new-s3cret!',
      confirmPassword: 'something-else!',
    })
    fixture.detectChanges()

    expect(fixture.nativeElement.textContent).not.toContain(
      'Les nouveaux mots de passe ne correspondent pas.',
    )
  })

  it('shows a passwords-mismatch message once the confirm field is touched', async () => {
    const { fixture } = await render('password')

    fixture.componentInstance.form.setValue({
      currentPassword: 'old-s3cret!',
      newPassword: 'new-s3cret!',
      confirmPassword: 'something-else!',
    })
    fixture.componentInstance.form.controls.confirmPassword.markAsTouched()
    fixture.detectChanges()

    expect(fixture.nativeElement.textContent).toContain(
      'Les nouveaux mots de passe ne correspondent pas.',
    )
  })

  it('clears the passwords-mismatch message once the values match again', async () => {
    const { fixture } = await render('password')

    fixture.componentInstance.form.setValue({
      currentPassword: 'old-s3cret!',
      newPassword: 'new-s3cret!',
      confirmPassword: 'something-else!',
    })
    fixture.componentInstance.form.controls.confirmPassword.markAsTouched()
    fixture.detectChanges()
    expect(fixture.nativeElement.textContent).toContain(
      'Les nouveaux mots de passe ne correspondent pas.',
    )

    fixture.componentInstance.form.controls.confirmPassword.setValue('new-s3cret!')
    fixture.detectChanges()

    expect(fixture.nativeElement.textContent).not.toContain(
      'Les nouveaux mots de passe ne correspondent pas.',
    )
  })

  it('does not show a too-short message before the new-password field is touched', async () => {
    const { fixture } = await render('password')

    fixture.componentInstance.form.setValue({
      currentPassword: 'old-s3cret!',
      newPassword: 'short',
      confirmPassword: 'short',
    })
    fixture.detectChanges()

    expect(fixture.nativeElement.textContent).not.toContain(
      'Le mot de passe doit contenir au moins 8 caractères.',
    )
  })

  it('shows a too-short message once the new-password field is touched with fewer than 8 characters', async () => {
    const { fixture } = await render('password')

    fixture.componentInstance.form.setValue({
      currentPassword: 'old-s3cret!',
      newPassword: 'short',
      confirmPassword: 'short',
    })
    fixture.componentInstance.form.controls.newPassword.markAsTouched()
    fixture.detectChanges()

    expect(fixture.nativeElement.textContent).toContain(
      'Le mot de passe doit contenir au moins 8 caractères.',
    )
  })

  it('does not load the API tokens outside their section', async () => {
    const { httpMock } = await render()

    httpMock.expectNone('/api/tokens')
  })

  it('shows the API tokens in their section', async () => {
    const { fixture, httpMock } = await render('tokens')
    httpMock.expectOne('/api/tokens').flush([])

    expect(fixture.debugElement.query(By.directive(ApiTokensList))).toBeTruthy()
  })

  describe("the security tab: Gabarit's settings on our API", () => {
    async function security() {
      const ctx = await render('security')
      ctx.httpMock
        .expectOne('/api/me/mfa')
        .flush({ totp_enabled: true, backup_codes_remaining: 6, passkey_count: 1 })
      ctx.httpMock.expectOne('/api/me/mfa/passkey').flush([
        {
          id: 'pk1',
          name: 'MacBook Touch ID',
          created_at: '2026-06-01T00:00:00Z',
          last_used_at: '2026-10-01T08:00:00Z',
        },
      ])
      ctx.httpMock
        .match('/api/auth/sso/config')
        .forEach((req) => req.flush({ type: null, registration_enabled: true }))
      ctx.fixture.detectChanges()
      return ctx
    }

    it('shows the authenticator app, its backup codes and the passkeys, in French', async () => {
      const { fixture } = await security()
      const text = fixture.nativeElement.textContent as string

      expect(fixture.debugElement.query(By.directive(MfaSettings))).toBeTruthy()
      expect(fixture.debugElement.query(By.directive(PasskeySettings))).toBeTruthy()
      expect(text).toContain('Application configurée')
      expect(text).toContain('6 codes de secours restants')
      expect(text).toContain('MacBook Touch ID')
    })

    it('offers to sign out everywhere, after the second factors', async () => {
      const { fixture } = await security()

      const cards = Array.from(
        fixture.nativeElement.querySelectorAll(
          'gbt-passkey-settings, gbt-mfa-settings, app-session-settings',
        ),
      ).map((element) => (element as Element).tagName.toLowerCase())
      expect(cards).toEqual(['gbt-passkey-settings', 'gbt-mfa-settings', 'app-session-settings'])
      expect(fixture.debugElement.query(By.directive(SessionSettings))).toBeTruthy()
    })

    it('signs out when a change on the server ended every session', async () => {
      const { fixture } = await security()

      fixture.debugElement.query(By.directive(MfaSettings)).componentInstance.sessionRevoked.emit()

      expect(signOutAndRedirect).toHaveBeenCalledTimes(1)
    })
  })
})
