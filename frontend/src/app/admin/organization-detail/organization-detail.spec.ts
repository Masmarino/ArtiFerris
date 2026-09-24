import { ComponentFixture, TestBed } from '@angular/core/testing'
import { By } from '@angular/platform-browser'
import { HttpErrorResponse } from '@angular/common/http'
import { Subject, of, throwError } from 'rxjs'
import { OrganizationDetail } from './organization-detail'
import { OrganizationsService } from '../application/organizations.service'
import { OrganizationMembersService } from '../application/organization-members.service'
import { OrganizationMembers } from '../organization-members/organization-members'
import { PageTitleService } from '../../shell/page-title.service'
import { ConfirmService } from '../../shared/confirm.service'
import { ToastService } from '../../shared/toast.service'
import { Tooltip } from '@masmarino/gabarit'

describe('OrganizationDetail', () => {
  let fixture: ComponentFixture<OrganizationDetail>
  let component: OrganizationDetail
  let organizationsServiceSpy: {
    get: ReturnType<typeof vi.fn>
    getIdentityProvider: ReturnType<typeof vi.fn>
    setLdapIdentityProvider: ReturnType<typeof vi.fn>
    setOidcIdentityProvider: ReturnType<typeof vi.fn>
    clearIdentityProvider: ReturnType<typeof vi.fn>
  }

  let askSpy: ReturnType<typeof vi.fn>

  beforeEach(() => {
    askSpy = vi.fn().mockResolvedValue(true)
  })

  function freshSpy() {
    // No jasmine here (vitest-based runner) — hand-rolled vi.fn() spies stand in.
    return {
      get: vi.fn(),
      getIdentityProvider: vi.fn(),
      setLdapIdentityProvider: vi.fn(),
      setOidcIdentityProvider: vi.fn(),
      clearIdentityProvider: vi.fn(),
    }
  }

  function render(organizationId = 'org-1') {
    TestBed.configureTestingModule({
      imports: [OrganizationDetail],
      providers: [
        { provide: OrganizationsService, useValue: organizationsServiceSpy },
        { provide: PageTitleService, useValue: { title: { set: vi.fn() } } },
        { provide: OrganizationMembersService, useValue: { list: () => of([]) } },
        { provide: ConfirmService, useValue: { ask: askSpy } },
      ],
    })
    fixture = TestBed.createComponent(OrganizationDetail)
    fixture.componentRef.setInput('organizationId', organizationId)
    component = fixture.componentInstance
    fixture.detectChanges()
  }

  function setup() {
    organizationsServiceSpy = freshSpy()
    organizationsServiceSpy.get.mockReturnValue(
      of({ id: 'org-1', slug: 'acme', display_name: 'Acme' }),
    )
    organizationsServiceSpy.getIdentityProvider.mockReturnValue(of({ type: null }))
    render()
  }

  it('loads the organization and its identity provider on init', () => {
    setup()
    expect(organizationsServiceSpy.get).toHaveBeenCalledWith('org-1')
    expect(organizationsServiceSpy.getIdentityProvider).toHaveBeenCalledWith('org-1')
    expect(component.organization()?.display_name).toBe('Acme')
    expect(component.identityProviderConfigured()).toBe(false)
  })

  it('shows an error and stops loading when fetching the organization fails', () => {
    organizationsServiceSpy = freshSpy()
    organizationsServiceSpy.get.mockReturnValue(throwError(() => new Error('load failed')))
    organizationsServiceSpy.getIdentityProvider.mockReturnValue(of({ type: null }))
    render()

    expect(component.loading()).toBe(false)
    expect(component.errorMessage()).toBe("Échec du chargement de l'organisation.")
    expect(component.organization()).toBeNull()
  })

  it('reflects an existing LDAP configuration', () => {
    organizationsServiceSpy = freshSpy()
    organizationsServiceSpy.get.mockReturnValue(
      of({ id: 'org-1', slug: 'acme', display_name: 'Acme' }),
    )
    organizationsServiceSpy.getIdentityProvider.mockReturnValue(
      of({
        type: 'ldap',
        server_url: 'ldap://dc.corp.example:389',
        bind_dn: 'cn=service,dc=corp,dc=example',
        bind_password_set: true,
        user_search_base: 'ou=people,dc=corp,dc=example',
        user_search_filter: '(uid={username})',
        email_attribute: 'mail',
      }),
    )
    render()

    expect(component.identityProviderConfigured()).toBe(true)
    expect(component.serverUrl()).toBe('ldap://dc.corp.example:389')
    expect(component.bindPasswordSet()).toBe(true)
  })

  it('explains via a tooltip what reverting to local accounts does, once an identity provider is configured', () => {
    organizationsServiceSpy = freshSpy()
    organizationsServiceSpy.get.mockReturnValue(
      of({ id: 'org-1', slug: 'acme', display_name: 'Acme' }),
    )
    organizationsServiceSpy.getIdentityProvider.mockReturnValue(
      of({
        type: 'ldap',
        server_url: 'ldap://dc.corp.example:389',
        bind_dn: 'cn=service,dc=corp,dc=example',
        bind_password_set: true,
        user_search_base: 'ou=people,dc=corp,dc=example',
        user_search_filter: '(uid={username})',
        email_attribute: 'mail',
      }),
    )
    render()

    const tooltip = fixture.debugElement.query(By.directive(Tooltip))

    expect((tooltip.componentInstance as Tooltip).text()).toBe(
      "Supprime la configuration du fournisseur d'identité et repasse cette organisation en comptes locaux.",
    )
  })

  it('saves the LDAP configuration', () => {
    setup()
    organizationsServiceSpy.setLdapIdentityProvider.mockReturnValue(of(undefined))
    component.serverUrl.set('ldap://dc.corp.example:389')
    component.bindDn.set('cn=service,dc=corp,dc=example')
    component.bindPassword.set('s3cret!')
    component.userSearchBase.set('ou=people,dc=corp,dc=example')
    component.userSearchFilter.set('(uid={username})')
    component.emailAttribute.set('mail')

    component.save()

    expect(organizationsServiceSpy.setLdapIdentityProvider).toHaveBeenCalledWith('org-1', {
      server_url: 'ldap://dc.corp.example:389',
      bind_dn: 'cn=service,dc=corp,dc=example',
      bind_password: 's3cret!',
      user_search_base: 'ou=people,dc=corp,dc=example',
      user_search_filter: '(uid={username})',
      email_attribute: 'mail',
    })
  })

  it('clears the LDAP configuration', async () => {
    setup()
    organizationsServiceSpy.clearIdentityProvider.mockReturnValue(of(undefined))

    await component.clear()

    expect(askSpy).toHaveBeenCalledWith(
      expect.objectContaining({ heading: 'Revenir aux comptes locaux' }),
    )
    expect(organizationsServiceSpy.clearIdentityProvider).toHaveBeenCalledWith('org-1')
  })

  it('shows the OIDC form fields when OIDC is selected as the provider type', () => {
    setup()
    component.selectedProviderType.set('oidc')
    fixture.detectChanges()

    expect(component.selectedProviderType()).toBe('oidc')
  })

  it('reflects an existing OIDC configuration', () => {
    organizationsServiceSpy = freshSpy()
    organizationsServiceSpy.get.mockReturnValue(
      of({ id: 'org-1', slug: 'acme', display_name: 'Acme' }),
    )
    organizationsServiceSpy.getIdentityProvider.mockReturnValue(
      of({
        type: 'oidc',
        issuer_url: 'https://accounts.example.com',
        client_id: 'artiferris',
        client_secret_set: true,
      }),
    )
    render()

    expect(component.identityProviderConfigured()).toBe(true)
    expect(component.selectedProviderType()).toBe('oidc')
    expect(component.issuerUrl()).toBe('https://accounts.example.com')
    expect(component.clientId()).toBe('artiferris')
    expect(component.clientSecretSet()).toBe(true)
  })

  it('saves the OIDC configuration', () => {
    setup()
    organizationsServiceSpy.setOidcIdentityProvider.mockReturnValue(of(undefined))
    component.selectedProviderType.set('oidc')
    component.issuerUrl.set('https://accounts.example.com')
    component.clientId.set('artiferris')
    component.clientSecret.set('s3cret!')

    component.save()

    expect(organizationsServiceSpy.setOidcIdentityProvider).toHaveBeenCalledWith('org-1', {
      issuer_url: 'https://accounts.example.com',
      client_id: 'artiferris',
      client_secret: 's3cret!',
    })
  })

  it('shows an error and does not optimistically mark the LDAP config as saved when save fails', () => {
    setup()
    organizationsServiceSpy.setLdapIdentityProvider.mockReturnValue(
      throwError(() => new Error('save failed')),
    )
    component.serverUrl.set('ldap://dc.corp.example:389')
    component.bindDn.set('cn=service,dc=corp,dc=example')
    component.bindPassword.set('s3cret!')
    component.userSearchBase.set('ou=people,dc=corp,dc=example')
    component.userSearchFilter.set('(uid={username})')
    component.emailAttribute.set('mail')

    const toastService = TestBed.inject(ToastService)
    component.save()

    expect(toastService.toasts().at(-1)).toMatchObject({
      variant: 'error',
      message: 'Échec de la mise à jour de la configuration.',
    })
    expect(component.saving()).toBe(false)
    expect(component.identityProviderConfigured()).toBe(false)
    expect(component.bindPasswordSet()).toBe(false)
  })

  it('shows an error and does not optimistically mark the OIDC config as saved when save fails', () => {
    setup()
    organizationsServiceSpy.setOidcIdentityProvider.mockReturnValue(
      throwError(() => new Error('save failed')),
    )
    component.selectedProviderType.set('oidc')
    component.issuerUrl.set('https://accounts.example.com')
    component.clientId.set('artiferris')
    component.clientSecret.set('s3cret!')

    const toastService = TestBed.inject(ToastService)
    component.save()

    expect(toastService.toasts().at(-1)).toMatchObject({
      variant: 'error',
      message: 'Échec de la mise à jour de la configuration.',
    })
    expect(component.saving()).toBe(false)
    expect(component.identityProviderConfigured()).toBe(false)
    expect(component.clientSecretSet()).toBe(false)
  })

  it('shows the server message when a kept LDAP secret is refused because the destination changed', () => {
    setup()
    organizationsServiceSpy.setLdapIdentityProvider.mockReturnValue(
      throwError(
        () =>
          new HttpErrorResponse({
            status: 400,
            error: {
              error: 're-enter the bind password when changing the server URL or the bind DN',
            },
          }),
      ),
    )
    component.serverUrl.set('ldaps://other.example:636')
    component.bindDn.set('cn=service,dc=corp,dc=example')
    component.bindPasswordSet.set(true)
    component.userSearchBase.set('ou=people,dc=corp,dc=example')
    component.userSearchFilter.set('(uid={username})')
    component.emailAttribute.set('mail')

    component.save()

    expect(TestBed.inject(ToastService).toasts().at(-1)).toMatchObject({
      variant: 'error',
      message: 're-enter the bind password when changing the server URL or the bind DN',
    })
  })

  it('shows the server message when a kept OIDC secret is refused because the destination changed', () => {
    setup()
    organizationsServiceSpy.setOidcIdentityProvider.mockReturnValue(
      throwError(
        () =>
          new HttpErrorResponse({
            status: 400,
            error: {
              error: 're-enter the client secret when changing the issuer URL or the client id',
            },
          }),
      ),
    )
    component.selectedProviderType.set('oidc')
    component.issuerUrl.set('https://other.example.com')
    component.clientId.set('artiferris')
    component.clientSecretSet.set(true)

    component.save()

    expect(TestBed.inject(ToastService).toasts().at(-1)).toMatchObject({
      message: 're-enter the client secret when changing the issuer URL or the client id',
    })
  })

  it('tells the admin to re-enter the secret, and shows the banner, on a 409', () => {
    setup()
    organizationsServiceSpy.setOidcIdentityProvider.mockReturnValue(
      throwError(
        () =>
          new HttpErrorResponse({
            status: 409,
            error: { error: 'a secret stored on the server cannot be read' },
          }),
      ),
    )
    component.selectedProviderType.set('oidc')
    component.issuerUrl.set('https://accounts.example.com')
    component.clientId.set('artiferris')
    component.clientSecret.set('s3cret!')

    component.save()

    expect(TestBed.inject(ToastService).toasts().at(-1)).toMatchObject({
      variant: 'error',
      message: 'Le secret enregistré est illisible : saisissez-le à nouveau',
    })
    expect(component.secretUnreadable()).toBe(true)
    expect(component.saving()).toBe(false)
  })

  it.each([
    [429, 'Trop de demandes, réessayez dans un instant'],
    [503, 'Service momentanément occupé'],
    [408, 'La requête a pris trop de temps, réessayez'],
  ])('words a %i in French', (status, message) => {
    setup()
    organizationsServiceSpy.setOidcIdentityProvider.mockReturnValue(
      throwError(() => new HttpErrorResponse({ status, error: { error: 'busy' } })),
    )
    component.selectedProviderType.set('oidc')
    component.issuerUrl.set('https://accounts.example.com')
    component.clientId.set('artiferris')
    component.clientSecret.set('s3cret!')

    component.save()

    expect(TestBed.inject(ToastService).toasts().at(-1)).toMatchObject({ message })
    expect(component.secretUnreadable()).toBe(false)
  })

  it('keeps the generic message for a failure that is not a 400', () => {
    setup()
    organizationsServiceSpy.setOidcIdentityProvider.mockReturnValue(
      throwError(() => new HttpErrorResponse({ status: 500, error: { error: 'internal error' } })),
    )
    component.selectedProviderType.set('oidc')
    component.issuerUrl.set('https://accounts.example.com')
    component.clientId.set('artiferris')
    component.clientSecret.set('s3cret!')

    component.save()

    expect(TestBed.inject(ToastService).toasts().at(-1)).toMatchObject({
      message: 'Échec de la mise à jour de la configuration.',
    })
  })

  describe('OIDC issuer', () => {
    function withOidcIssuer(url: string) {
      setup()
      component.selectedProviderType.set('oidc')
      component.issuerUrl.set(url)
      component.clientId.set('artiferris')
      component.clientSecret.set('s3cret!')
      fixture.detectChanges()
    }

    it('refuses an http issuer, shows why, and does not send it', () => {
      withOidcIssuer('http://accounts.example.com')

      expect(component.issuerError()).toBe("L'URL de l'émetteur doit commencer par https://.")
      expect(component.hasErrors()).toBe(true)
      expect(fixture.nativeElement.textContent).toContain('doit commencer par https://')
      component.save()
      expect(organizationsServiceSpy.setOidcIdentityProvider).not.toHaveBeenCalled()
    })

    it('accepts an https issuer whatever its case', () => {
      withOidcIssuer('HTTPS://Accounts.Example.com')

      expect(component.issuerError()).toBeNull()
      expect(component.hasErrors()).toBe(false)
    })
  })

  describe('a stored secret that cannot be decrypted', () => {
    function renderUnreadable() {
      organizationsServiceSpy = freshSpy()
      organizationsServiceSpy.get.mockReturnValue(
        of({ id: 'org-1', slug: 'acme', display_name: 'Acme' }),
      )
      organizationsServiceSpy.getIdentityProvider.mockReturnValue(
        of({
          type: null,
          secret_unreadable: true,
          error: "the stored secret cannot be read with this server's SECRETS_ENCRYPTION_KEY",
        }),
      )
      render()
    }
    const banner = () => fixture.nativeElement.querySelector('gbt-alert')

    it('warns that the provider must be configured again', () => {
      renderUnreadable()

      expect(component.secretUnreadable()).toBe(true)
      expect(banner().querySelector('[data-variant="warning"]')).not.toBeNull()
      expect(banner().textContent).toContain('illisible')
      expect(banner().textContent).toContain('Configurez à nouveau le fournisseur')
      expect(fixture.nativeElement.textContent).not.toContain('utilise des comptes locaux')
    })

    it('still requires the secret again before saving', () => {
      renderUnreadable()
      component.serverUrl.set('ldaps://dc.corp.example:636')
      component.bindDn.set('cn=service,dc=corp,dc=example')
      component.userSearchBase.set('ou=people,dc=corp,dc=example')
      component.userSearchFilter.set('(uid={username})')
      component.emailAttribute.set('mail')

      expect(component.hasErrors()).toBe(true)
    })

    it('stays offered the way back to local accounts', () => {
      renderUnreadable()

      expect(fixture.debugElement.queryAll(By.directive(Tooltip)).length).toBe(1)
    })

    it('drops the warning once the provider has been saved again', () => {
      renderUnreadable()
      organizationsServiceSpy.setLdapIdentityProvider.mockReturnValue(of(undefined))
      component.serverUrl.set('ldaps://dc.corp.example:636')
      component.bindDn.set('cn=service,dc=corp,dc=example')
      component.bindPassword.set('s3cret!')
      component.userSearchBase.set('ou=people,dc=corp,dc=example')
      component.userSearchFilter.set('(uid={username})')
      component.emailAttribute.set('mail')

      component.save()
      fixture.detectChanges()

      expect(component.secretUnreadable()).toBe(false)
      expect(banner()).toBeNull()
    })

    it('shows no warning for a provider that is simply not configured', () => {
      setup()

      expect(component.secretUnreadable()).toBe(false)
      expect(banner()).toBeNull()
    })
  })

  it('shows an error and does not reset the form when clearing the configuration fails', async () => {
    organizationsServiceSpy = freshSpy()
    organizationsServiceSpy.get.mockReturnValue(
      of({ id: 'org-1', slug: 'acme', display_name: 'Acme' }),
    )
    organizationsServiceSpy.getIdentityProvider.mockReturnValue(
      of({
        type: 'ldap',
        server_url: 'ldap://dc.corp.example:389',
        bind_dn: 'cn=service,dc=corp,dc=example',
        bind_password_set: true,
        user_search_base: 'ou=people,dc=corp,dc=example',
        user_search_filter: '(uid={username})',
        email_attribute: 'mail',
      }),
    )
    render()

    organizationsServiceSpy.clearIdentityProvider.mockReturnValue(
      throwError(() => new Error('clear failed')),
    )

    const toastService = TestBed.inject(ToastService)
    await component.clear()

    expect(toastService.toasts().at(-1)).toMatchObject({
      variant: 'error',
      message: 'Échec de la suppression de la configuration.',
    })
    expect(component.clearing()).toBe(false)
    expect(component.identityProviderConfigured()).toBe(true)
    expect(component.serverUrl()).toBe('ldap://dc.corp.example:389')
    expect(component.bindPasswordSet()).toBe(true)
  })

  it('renders the members component with the resolved organization id', () => {
    setup()

    const membersDebugElement = fixture.debugElement.query(By.directive(OrganizationMembers))
    expect(membersDebugElement).not.toBeNull()
    expect(membersDebugElement.componentInstance.organizationId()).toBe('org-1')
  })

  it('re-fetches (and resets stale identity-provider fields) when organizationId changes to a different organization — the component is reused, not recreated, across a super-admin switching organizations', () => {
    organizationsServiceSpy = freshSpy()
    organizationsServiceSpy.get.mockReturnValue(
      of({ id: 'org-1', slug: 'acme', display_name: 'Acme' }),
    )
    organizationsServiceSpy.getIdentityProvider.mockReturnValue(
      of({
        type: 'ldap',
        server_url: 'ldap://dc.corp.example:389',
        bind_dn: 'cn=service,dc=corp,dc=example',
        bind_password_set: true,
        user_search_base: 'ou=people,dc=corp,dc=example',
        user_search_filter: '(uid={username})',
        email_attribute: 'mail',
      }),
    )
    render('org-1')
    expect(component.identityProviderConfigured()).toBe(true)
    expect(component.serverUrl()).toBe('ldap://dc.corp.example:389')

    // org-2 has no identity provider of its own — must not still show org-1's configuration.
    organizationsServiceSpy.get.mockReturnValue(
      of({ id: 'org-2', slug: 'other', display_name: 'Other' }),
    )
    organizationsServiceSpy.getIdentityProvider.mockReturnValue(of({ type: null }))
    fixture.componentRef.setInput('organizationId', 'org-2')
    fixture.detectChanges()

    expect(organizationsServiceSpy.get).toHaveBeenCalledWith('org-2')
    expect(component.identityProviderConfigured()).toBe(false)
    expect(component.serverUrl()).toBe('')
    expect(component.bindPasswordSet()).toBe(false)
  })

  describe('identity provider lookup failure', () => {
    const LDAP = {
      type: 'ldap',
      server_url: 'ldap://dc.corp.example:389',
      bind_dn: 'cn=service,dc=corp,dc=example',
      bind_password_set: true,
      user_search_base: 'ou=people,dc=corp,dc=example',
      user_search_filter: '(uid={username})',
      email_attribute: 'mail',
    }

    it('shows an error instead of a form that could overwrite the provider', () => {
      organizationsServiceSpy = freshSpy()
      organizationsServiceSpy.get.mockReturnValue(
        of({ id: 'org-1', slug: 'acme', display_name: 'Acme' }),
      )
      organizationsServiceSpy.getIdentityProvider.mockReturnValue(
        throwError(() => new Error('boom')),
      )
      render()

      expect(component.errorMessage()).toBe("Échec du chargement de l'organisation.")
      expect(component.organization()).toBeNull()
      expect(fixture.nativeElement.textContent).not.toContain('Enregistrer')
      expect(fixture.nativeElement.textContent).not.toContain('comptes locaux')
      expect(fixture.nativeElement.querySelector('[role="alert"]')).toBeTruthy()
    })

    it('loads the form after a retry succeeds', () => {
      organizationsServiceSpy = freshSpy()
      organizationsServiceSpy.get.mockReturnValue(
        of({ id: 'org-1', slug: 'acme', display_name: 'Acme' }),
      )
      organizationsServiceSpy.getIdentityProvider.mockReturnValue(
        throwError(() => new Error('boom')),
      )
      render()

      organizationsServiceSpy.getIdentityProvider.mockReturnValue(of(LDAP))
      const retry = Array.from<HTMLButtonElement>(
        fixture.nativeElement.querySelectorAll('button'),
      ).find((button) => button.textContent?.includes('Réessayer'))!
      retry.click()
      fixture.detectChanges()

      expect(component.errorMessage()).toBeNull()
      expect(component.serverUrl()).toBe('ldap://dc.corp.example:389')
      expect(fixture.nativeElement.textContent).toContain('Enregistrer')
    })

    it("never shows organization A's provider under organization B when B's lookup fails", () => {
      organizationsServiceSpy = freshSpy()
      organizationsServiceSpy.get.mockReturnValue(
        of({ id: 'org-1', slug: 'acme', display_name: 'Acme' }),
      )
      organizationsServiceSpy.getIdentityProvider.mockReturnValue(of(LDAP))
      render('org-1')
      expect(component.serverUrl()).toBe('ldap://dc.corp.example:389')

      organizationsServiceSpy.get.mockReturnValue(
        of({ id: 'org-2', slug: 'other', display_name: 'Other' }),
      )
      organizationsServiceSpy.getIdentityProvider.mockReturnValue(
        throwError(() => new Error('boom')),
      )
      fixture.componentRef.setInput('organizationId', 'org-2')
      fixture.detectChanges()

      expect(component.serverUrl()).toBe('')
      expect(component.identityProviderConfigured()).toBe(false)
      expect(component.organization()).toBeNull()
      expect(component.errorMessage()).not.toBeNull()
    })
  })

  describe('switching organization while requests are in flight', () => {
    it("drops organization A's late answers and clears its form while B loads", () => {
      organizationsServiceSpy = freshSpy()
      const organizationA = new Subject<unknown>()
      const providerA = new Subject<unknown>()
      organizationsServiceSpy.get.mockReturnValue(organizationA)
      organizationsServiceSpy.getIdentityProvider.mockReturnValue(providerA)
      render('org-1')

      const organizationB = new Subject<unknown>()
      const providerB = new Subject<unknown>()
      organizationsServiceSpy.get.mockReturnValue(organizationB)
      organizationsServiceSpy.getIdentityProvider.mockReturnValue(providerB)
      fixture.componentRef.setInput('organizationId', 'org-2')
      fixture.detectChanges()

      const answer = (subject: Subject<unknown>, value: unknown) => {
        subject.next(value)
        subject.complete()
      }
      answer(organizationB, { id: 'org-2', slug: 'other', display_name: 'Other' })
      answer(providerB, { type: null })
      answer(organizationA, { id: 'org-1', slug: 'acme', display_name: 'Acme' })
      answer(providerA, {
        type: 'oidc',
        issuer_url: 'https://idp.acme.example',
        client_id: 'acme',
        client_secret_set: true,
      })
      fixture.detectChanges()

      expect(component.organization()?.display_name).toBe('Other')
      expect(component.identityProviderConfigured()).toBe(false)
      expect(component.issuerUrl()).toBe('')
    })

    it('does not show the previous organization while the next one loads', () => {
      organizationsServiceSpy = freshSpy()
      organizationsServiceSpy.get.mockReturnValue(
        of({ id: 'org-1', slug: 'acme', display_name: 'Acme' }),
      )
      organizationsServiceSpy.getIdentityProvider.mockReturnValue(of({ type: null }))
      render('org-1')
      expect(fixture.nativeElement.textContent).toContain('Acme')

      organizationsServiceSpy.get.mockReturnValue(new Subject())
      fixture.componentRef.setInput('organizationId', 'org-2')
      fixture.detectChanges()

      expect(component.loading()).toBe(true)
      expect(component.organization()).toBeNull()
      expect(fixture.nativeElement.textContent).not.toContain('Acme')
    })
  })
})
