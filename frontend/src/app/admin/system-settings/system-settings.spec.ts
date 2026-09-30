import { signal } from '@angular/core'
import { ComponentFixture, TestBed } from '@angular/core/testing'
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing'
import { provideHttpClient } from '@angular/common/http'
import { SystemSettingsAdmin } from './system-settings'
import { adminProviders } from '../infrastructure/admin.providers'
import { ToastService } from '../../shared/toast.service'
import { MeService } from '../../shell/application/me.service'
import { PUBLIC_ORGANIZATION_ID } from '../domain/organization.entity'

const SEO_LABEL = "Autoriser l'indexation par les moteurs de recherche"

interface RenderOptions {
  isSuperAdmin?: boolean
  ownOrganizationId?: string | null
  organizationId?: string
  seoIndexingEnabled?: boolean
  publicPageEnabled?: boolean
  seoIndexingBlocked?: boolean
}

function render(options: RenderOptions = {}) {
  const { isSuperAdmin = false, ownOrganizationId = null, organizationId } = options
  TestBed.configureTestingModule({
    providers: [
      provideHttpClient(),
      provideHttpClientTesting(),
      ...adminProviders,
      {
        provide: MeService,
        useValue: { isSuperAdmin: signal(isSuperAdmin), organizationId: signal(ownOrganizationId) },
      },
    ],
  })
  const fixture = TestBed.createComponent(SystemSettingsAdmin)
  if (organizationId) {
    fixture.componentRef.setInput('organizationId', organizationId)
  }
  const httpMock = TestBed.inject(HttpTestingController)
  fixture.detectChanges()
  httpMock
    .expectOne((req) => req.url === '/api/admin/settings')
    .flush({
      max_login_attempts: 10,
      login_attempt_window_seconds: 300,
      session_ttl_hours: 12,
      registration_enabled: true,
      seo_indexing_enabled: options.seoIndexingEnabled ?? false,
      seo_indexing_blocked: options.seoIndexingBlocked ?? false,
      public_page_enabled: options.publicPageEnabled ?? true,
    })
  fixture.detectChanges()
  return { fixture, httpMock }
}

describe('SystemSettingsAdmin', () => {
  it('loads the current settings into the form', () => {
    const { fixture } = render()

    expect(fixture.componentInstance.maxLoginAttempts()).toBe('10')
    expect(fixture.componentInstance.loginAttemptWindowSeconds()).toBe('300')
    expect(fixture.componentInstance.sessionTtlHours()).toBe('12')
  })

  it('saves valid values', () => {
    const { fixture, httpMock } = render()
    fixture.componentInstance.setValue('maxLoginAttempts', '5')

    fixture.componentInstance.save()

    const req = httpMock.expectOne('/api/admin/settings')
    expect(req.request.method).toBe('PUT')
    expect(req.request.body).toEqual({
      max_login_attempts: 5,
      login_attempt_window_seconds: 300,
      session_ttl_hours: 12,
      registration_enabled: true,
      seo_indexing_enabled: false,
      seo_indexing_blocked: false,
      public_page_enabled: true,
    })
    const toastService = TestBed.inject(ToastService)
    req.flush(null)
    fixture.detectChanges()

    expect(toastService.toasts().at(-1)).toMatchObject({
      variant: 'success',
      message: 'Paramètres enregistrés.',
    })
  })

  it('saves the registration toggle when turned off', () => {
    const { fixture, httpMock } = render()
    fixture.componentInstance.registrationEnabled.set(false)

    fixture.componentInstance.save()

    const req = httpMock.expectOne('/api/admin/settings')
    expect(req.request.body).toEqual({
      max_login_attempts: 10,
      login_attempt_window_seconds: 300,
      session_ttl_hours: 12,
      registration_enabled: false,
      seo_indexing_enabled: false,
      seo_indexing_blocked: false,
      public_page_enabled: true,
    })
    req.flush(null)
  })

  describe('public page controls', () => {
    const checkbox = (fixture: ComponentFixture<SystemSettingsAdmin>, label: string) =>
      Array.from(fixture.nativeElement.querySelectorAll('gbt-checkbox')).find((box) =>
        (box as HTMLElement).textContent?.includes(label),
      ) as HTMLElement | undefined

    it("offers an organization admin to close the organization's page and keep search engines away", () => {
      const { fixture } = render({ ownOrganizationId: 'org-acme' })

      expect(fixture.nativeElement.textContent).toContain('Page publique')
      expect(checkbox(fixture, "Afficher la page publique de l'organisation")).toBeDefined()
      expect(checkbox(fixture, 'Bloquer les moteurs de recherche pour cette organisation')).toBeDefined()
    })

    it("offers the instance's pages, and nothing about one organization's crawlers, to a super-admin on the public organization", () => {
      const { fixture } = render({ isSuperAdmin: true, ownOrganizationId: PUBLIC_ORGANIZATION_ID })

      expect(checkbox(fixture, "Afficher les pages publiques de l'instance")).toBeDefined()
      expect(checkbox(fixture, 'Bloquer les moteurs de recherche')).toBeUndefined()
    })

    it("does not offer closing the instance's pages to an admin who is not a super-admin", () => {
      const { fixture } = render({ isSuperAdmin: false, ownOrganizationId: PUBLIC_ORGANIZATION_ID })

      expect(fixture.nativeElement.textContent).not.toContain('Page publique')
    })

    it('loads the stored values and sends the changed ones on save', () => {
      const { fixture, httpMock } = render({
        ownOrganizationId: 'org-acme',
        publicPageEnabled: false,
        seoIndexingBlocked: true,
      })
      expect(fixture.componentInstance.publicPageEnabled()).toBe(false)
      expect(fixture.componentInstance.seoIndexingBlocked()).toBe(true)

      fixture.componentInstance.publicPageEnabled.set(true)
      fixture.componentInstance.seoIndexingBlocked.set(false)
      fixture.componentInstance.save()

      const req = httpMock.expectOne((r) => r.url === '/api/admin/settings')
      expect(req.request.body.public_page_enabled).toBe(true)
      expect(req.request.body.seo_indexing_blocked).toBe(false)
      req.flush(null)
    })

    it('keeps the loaded values when the section is hidden', () => {
      const { fixture, httpMock } = render({
        isSuperAdmin: false,
        ownOrganizationId: PUBLIC_ORGANIZATION_ID,
        publicPageEnabled: false,
        seoIndexingBlocked: true,
      })

      fixture.componentInstance.save()

      const req = httpMock.expectOne((r) => r.url === '/api/admin/settings')
      expect(req.request.body.public_page_enabled).toBe(false)
      expect(req.request.body.seo_indexing_blocked).toBe(true)
      req.flush(null)
    })
  })

  describe('search-engine indexing', () => {
    const publicOrg = { isSuperAdmin: true, organizationId: PUBLIC_ORGANIZATION_ID }

    function seoInput(fixture: ReturnType<typeof render>['fixture']) {
      const checkbox = Array.from(
        (fixture.nativeElement as HTMLElement).querySelectorAll('gbt-checkbox'),
      ).find((el) => el.textContent?.includes(SEO_LABEL))
      return checkbox?.querySelector('input') ?? null
    }

    it('shows the section to a super-admin on their own public organization', () => {
      const { fixture } = render({ isSuperAdmin: true, ownOrganizationId: PUBLIC_ORGANIZATION_ID })

      expect(seoInput(fixture)).not.toBeNull()
    })

    it('shows the section when the public organization is targeted by id', () => {
      const { fixture } = render(publicOrg)

      expect(fixture.nativeElement.textContent).toContain('Référencement')
      expect(fixture.nativeElement.textContent).toContain('« noindex »')
      expect(seoInput(fixture)).not.toBeNull()
    })

    it('scopes the section to the explicit organization over the user own one', () => {
      const { fixture } = render({
        isSuperAdmin: true,
        ownOrganizationId: PUBLIC_ORGANIZATION_ID,
        organizationId: 'org-acme',
      })

      expect(seoInput(fixture)).toBeNull()
    })

    it('hides the section from a non-super-admin on the public organization', () => {
      const { fixture } = render({ isSuperAdmin: false, ownOrganizationId: PUBLIC_ORGANIZATION_ID })

      expect(fixture.nativeElement.textContent).not.toContain('Référencement')
      expect(seoInput(fixture)).toBeNull()
    })

    it('hides the section on another organization', () => {
      const { fixture } = render({ isSuperAdmin: true, ownOrganizationId: 'org-acme' })

      expect(fixture.nativeElement.textContent).not.toContain('Référencement')
      expect(seoInput(fixture)).toBeNull()
    })

    it('reflects the loaded value in the checkbox', async () => {
      const { fixture } = render({ ...publicOrg, seoIndexingEnabled: true })
      await fixture.whenStable()
      fixture.detectChanges()

      expect(seoInput(fixture)?.checked).toBe(true)
    })

    it('sends the toggled value on save', () => {
      const { fixture, httpMock } = render(publicOrg)
      const input = seoInput(fixture)!

      input.click()
      fixture.detectChanges()
      fixture.componentInstance.save()

      const req = httpMock.expectOne((r) => r.url === '/api/admin/settings')
      expect(req.request.body.seo_indexing_enabled).toBe(true)
      req.flush(null)
    })

    it('keeps the loaded value when the section is hidden', () => {
      const { fixture, httpMock } = render({ isSuperAdmin: false, seoIndexingEnabled: true })

      fixture.componentInstance.save()

      const req = httpMock.expectOne((r) => r.url === '/api/admin/settings')
      expect(req.request.body.seo_indexing_enabled).toBe(true)
      req.flush(null)
    })

    it('keeps the loaded value on another organization scoped save', () => {
      const { fixture, httpMock } = render({
        isSuperAdmin: true,
        organizationId: 'org-acme',
        seoIndexingEnabled: true,
      })

      fixture.componentInstance.save()

      const req = httpMock.expectOne((r) => r.url === '/api/admin/settings')
      expect(req.request.params.get('organization_id')).toBe('org-acme')
      expect(req.request.body.seo_indexing_enabled).toBe(true)
      req.flush(null)
    })
  })

  it('rejects an out-of-range value without sending a request', () => {
    const { fixture, httpMock } = render()
    fixture.componentInstance.setValue('maxLoginAttempts', '0')
    fixture.detectChanges()

    fixture.componentInstance.save()

    httpMock.expectNone('/api/admin/settings')
    expect(fixture.componentInstance.hasErrors()).toBe(true)
  })

  it('hides a field error until a save is attempted, then reveals it', () => {
    const { fixture } = render()
    fixture.componentInstance.setValue('maxLoginAttempts', '0')
    const field = fixture.componentInstance.fields.find((f) => f.key === 'maxLoginAttempts')!

    expect(fixture.componentInstance.fieldError(field)).toBeNull()
    expect(fixture.componentInstance.hasErrors()).toBe(false)

    fixture.componentInstance.save()

    expect(fixture.componentInstance.fieldError(field)).toContain('entre')
    expect(fixture.componentInstance.hasErrors()).toBe(true)
  })

  it('rejects a non-integer value', () => {
    const { fixture } = render()
    fixture.componentInstance.setValue('sessionTtlHours', 'abc')
    // Errors stay hidden until a save is attempted.
    fixture.componentInstance.save()

    const field = fixture.componentInstance.fields.find((f) => f.key === 'sessionTtlHours')!
    expect(fixture.componentInstance.fieldError(field)).toContain('entier')
  })

  it('shows a generic error toast when the save request fails', () => {
    const { fixture, httpMock } = render()

    fixture.componentInstance.save()

    const toastService = TestBed.inject(ToastService)
    httpMock
      .expectOne('/api/admin/settings')
      .flush(null, { status: 400, statusText: 'Bad Request' })
    fixture.detectChanges()

    expect(toastService.toasts().at(-1)).toMatchObject({
      variant: 'error',
      message: 'Échec de la mise à jour des paramètres.',
    })
  })

  describe('switching organizations', () => {
    const settings = (attempts: number) => ({
      max_login_attempts: attempts,
      login_attempt_window_seconds: 300,
      session_ttl_hours: 12,
      registration_enabled: true,
      seo_indexing_enabled: false,
      seo_indexing_blocked: false,
      public_page_enabled: true,
    })

    function switchTwice() {
      const { fixture, httpMock } = render()
      fixture.componentRef.setInput('organizationId', 'org-a')
      fixture.detectChanges()
      const forA = httpMock.expectOne('/api/admin/settings?organization_id=org-a')
      fixture.componentRef.setInput('organizationId', 'org-b')
      fixture.detectChanges()
      const forB = httpMock.expectOne('/api/admin/settings?organization_id=org-b')
      return { fixture, httpMock, forA, forB }
    }

    it('keeps the current organization when a slower response for the previous one lands last', () => {
      const { fixture, httpMock, forA, forB } = switchTwice()

      forB.flush(settings(2))
      forA.flush(settings(99))
      fixture.detectChanges()

      expect(fixture.componentInstance.maxLoginAttempts()).toBe('2')
      fixture.componentInstance.save()
      expect(
        httpMock.expectOne('/api/admin/settings?organization_id=org-b').request.body,
      ).toMatchObject({ max_login_attempts: 2 })
    })

    it('shows an error, not an endless spinner, when the load fails', () => {
      const { fixture, forA, forB } = switchTwice()

      forA.flush(null, { status: 500, statusText: 'boom' })
      forB.flush(null, { status: 500, statusText: 'boom' })
      fixture.detectChanges()

      expect(fixture.componentInstance.loading()).toBe(false)
      expect(fixture.nativeElement.querySelector('gbt-spinner')).toBeNull()
      expect(fixture.nativeElement.textContent).toContain('Échec du chargement des paramètres.')
    })

    it('ignores a failure of the request for the organization the user already left', () => {
      const { fixture, forA, forB } = switchTwice()

      forB.flush(settings(2))
      forA.flush(null, { status: 500, statusText: 'boom' })
      fixture.detectChanges()

      expect(fixture.componentInstance.loadFailed()).toBe(false)
    })
  })
})
