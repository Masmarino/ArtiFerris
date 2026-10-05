import { TestBed } from '@angular/core/testing'
import { By } from '@angular/platform-browser'
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing'
import { provideHttpClient } from '@angular/common/http'
import { Tooltip } from '@masmarino/gabarit/tooltip'
import { SmtpSettingsAdmin } from './smtp-settings'
import { adminProviders } from '../infrastructure/admin.providers'
import { ToastService } from '../../shared/toast.service'

function render(existing: object | null = null) {
  TestBed.configureTestingModule({
    providers: [provideHttpClient(), provideHttpClientTesting(), ...adminProviders],
  })
  const fixture = TestBed.createComponent(SmtpSettingsAdmin)
  const httpMock = TestBed.inject(HttpTestingController)
  fixture.detectChanges()
  httpMock.expectOne('/api/admin/settings/smtp').flush(existing)
  fixture.detectChanges()
  return { fixture, httpMock }
}

describe('SmtpSettingsAdmin', () => {
  it('starts with an empty form when SMTP has never been configured', () => {
    const { fixture } = render(null)

    expect(fixture.componentInstance.host()).toBe('')
    expect(fixture.componentInstance.passwordSet()).toBe(false)
  })

  it('shows no field errors and does not disable Enregistrer before any save attempt, even though every required field starts empty', () => {
    const { fixture } = render(null)

    expect(fixture.componentInstance.hostError()).toBeNull()
    expect(fixture.componentInstance.usernameError()).toBeNull()
    expect(fixture.componentInstance.passwordError()).toBeNull()
    expect(fixture.componentInstance.fromAddressError()).toBeNull()
    expect(fixture.componentInstance.hasErrors()).toBe(false)
  })

  it('reveals the field errors once a save is attempted on an invalid form', () => {
    const { fixture, httpMock } = render(null)

    fixture.componentInstance.save()

    httpMock.expectNone('/api/admin/settings/smtp')
    expect(fixture.componentInstance.hostError()).toBe("L'hôte est requis.")
    expect(fixture.componentInstance.usernameError()).toBe("L'identifiant est requis.")
  })

  it('loads existing settings into the form without ever receiving the password', () => {
    const { fixture } = render({
      host: 'smtp.example.com',
      port: 587,
      username: 'artiferris@example.com',
      from_name: 'Acme Corp',
      from_address: 'artiferris@example.com',
      security: 'start_tls',
      password_set: true,
    })

    expect(fixture.componentInstance.host()).toBe('smtp.example.com')
    expect(fixture.componentInstance.port()).toBe('587')
    expect(fixture.componentInstance.fromName()).toBe('Acme Corp')
    expect(fixture.componentInstance.passwordSet()).toBe(true)
    expect(fixture.componentInstance.password()).toBe('')
  })

  it('requires a password on first-time configuration', () => {
    const { fixture, httpMock } = render(null)
    fixture.componentInstance.host.set('smtp.example.com')
    fixture.componentInstance.username.set('artiferris@example.com')
    fixture.componentInstance.fromAddress.set('artiferris@example.com')

    fixture.componentInstance.save()

    httpMock.expectNone('/api/admin/settings/smtp')
    expect(fixture.componentInstance.hasErrors()).toBe(true)
  })

  it('saves valid settings and leaves the password blank afterwards', () => {
    const { fixture, httpMock } = render(null)
    fixture.componentInstance.host.set('smtp.example.com')
    fixture.componentInstance.username.set('artiferris@example.com')
    fixture.componentInstance.password.set('s3cret')
    fixture.componentInstance.fromAddress.set('artiferris@example.com')

    fixture.componentInstance.save()

    const req = httpMock.expectOne('/api/admin/settings/smtp')
    expect(req.request.method).toBe('PUT')
    expect(req.request.body).toEqual({
      host: 'smtp.example.com',
      port: 587,
      username: 'artiferris@example.com',
      password: 's3cret',
      from_name: 'ArtiFerris',
      from_address: 'artiferris@example.com',
      security: 'start_tls',
    })
    const toastService = TestBed.inject(ToastService)
    req.flush(null)
    fixture.detectChanges()

    expect(toastService.toasts().at(-1)).toMatchObject({
      variant: 'success',
      message: 'Paramètres SMTP enregistrés.',
    })
    expect(fixture.componentInstance.password()).toBe('')
    expect(fixture.componentInstance.passwordSet()).toBe(true)
  })

  it('omits the password from the update request when left blank on an already-configured instance', () => {
    const { fixture, httpMock } = render({
      host: 'smtp.example.com',
      port: 587,
      username: 'artiferris@example.com',
      from_name: 'ArtiFerris',
      from_address: 'artiferris@example.com',
      security: 'start_tls',
      password_set: true,
    })

    fixture.componentInstance.save()

    const req = httpMock.expectOne('/api/admin/settings/smtp')
    expect(req.request.body.password).toBeUndefined()
    req.flush(null)
  })

  it('rejects a from_address without an @ sign', () => {
    const { fixture, httpMock } = render(null)
    fixture.componentInstance.host.set('smtp.example.com')
    fixture.componentInstance.username.set('artiferris@example.com')
    fixture.componentInstance.password.set('s3cret')
    fixture.componentInstance.fromAddress.set('not-an-email')

    fixture.componentInstance.save()

    httpMock.expectNone('/api/admin/settings/smtp')
    expect(fixture.componentInstance.hasErrors()).toBe(true)
  })

  it('rejects an empty from name', () => {
    const { fixture, httpMock } = render(null)
    fixture.componentInstance.host.set('smtp.example.com')
    fixture.componentInstance.username.set('artiferris@example.com')
    fixture.componentInstance.password.set('s3cret')
    fixture.componentInstance.fromAddress.set('artiferris@example.com')
    fixture.componentInstance.fromName.set('  ')

    fixture.componentInstance.save()

    httpMock.expectNone('/api/admin/settings/smtp')
    expect(fixture.componentInstance.hasErrors()).toBe(true)
  })

  it('shows a generic error toast when the save request fails', () => {
    const { fixture, httpMock } = render(null)
    fixture.componentInstance.host.set('smtp.example.com')
    fixture.componentInstance.username.set('artiferris@example.com')
    fixture.componentInstance.password.set('s3cret')
    fixture.componentInstance.fromAddress.set('artiferris@example.com')

    fixture.componentInstance.save()

    const toastService = TestBed.inject(ToastService)
    httpMock
      .expectOne('/api/admin/settings/smtp')
      .flush(null, { status: 400, statusText: 'Bad Request' })
    fixture.detectChanges()

    expect(toastService.toasts().at(-1)).toMatchObject({
      variant: 'error',
      message: 'Échec de la mise à jour des paramètres SMTP.',
    })
  })

  it('tells the admin to re-enter the password, and shows the banner, when the save answers a 409', () => {
    const { fixture, httpMock } = render(null)
    fixture.componentInstance.host.set('smtp.example.com')
    fixture.componentInstance.username.set('artiferris@example.com')
    fixture.componentInstance.password.set('s3cret')
    fixture.componentInstance.fromAddress.set('artiferris@example.com')

    fixture.componentInstance.save()
    httpMock
      .expectOne('/api/admin/settings/smtp')
      .flush(
        { error: 'a secret stored on the server cannot be read' },
        { status: 409, statusText: 'Conflict' },
      )
    fixture.detectChanges()

    expect(TestBed.inject(ToastService).toasts().at(-1)).toMatchObject({
      variant: 'error',
      message: 'Le secret enregistré est illisible : saisissez-le à nouveau',
    })
    expect(fixture.componentInstance.secretUnreadable()).toBe(true)
  })

  it.each([
    [429, 'Trop de demandes, réessayez dans un instant'],
    [503, 'Service momentanément occupé'],
  ])('words a %i on save in French', (status, message) => {
    const { fixture, httpMock } = render(null)
    fixture.componentInstance.host.set('smtp.example.com')
    fixture.componentInstance.username.set('artiferris@example.com')
    fixture.componentInstance.password.set('s3cret')
    fixture.componentInstance.fromAddress.set('artiferris@example.com')

    fixture.componentInstance.save()
    httpMock
      .expectOne('/api/admin/settings/smtp')
      .flush({ error: 'x' }, { status, statusText: 'x' })

    expect(TestBed.inject(ToastService).toasts().at(-1)).toMatchObject({ message })
  })

  it('shows the same unreadable-secret message when the test e-mail answers a 409', () => {
    const { fixture, httpMock } = render({
      host: 'smtp.example.com',
      port: 587,
      username: 'artiferris@example.com',
      from_name: 'ArtiFerris',
      from_address: 'artiferris@example.com',
      security: 'start_tls',
      password_set: true,
    })
    fixture.componentInstance.testRecipient.set('admin@example.com')

    fixture.componentInstance.sendTest()
    httpMock
      .expectOne('/api/admin/settings/smtp/test')
      .flush({}, { status: 409, statusText: 'Conflict' })

    expect(TestBed.inject(ToastService).toasts().at(-1)).toMatchObject({
      message: 'Le secret enregistré est illisible : saisissez-le à nouveau',
    })
    expect(fixture.componentInstance.secretUnreadable()).toBe(true)
  })

  it('shows the server message when a kept password is refused because host, port or username changed', () => {
    const { fixture, httpMock } = render({
      host: 'smtp.example.com',
      port: 587,
      username: 'relay',
      from_name: 'ArtiFerris',
      from_address: 'artiferris@example.com',
      security: 'start_tls',
      password_set: true,
    })
    fixture.componentInstance.host.set('other.example.net')

    fixture.componentInstance.save()
    httpMock.expectOne('/api/admin/settings/smtp').flush(
      {
        error: 're-enter the password when changing the host, port or username',
      },
      { status: 400, statusText: 'Bad Request' },
    )

    expect(TestBed.inject(ToastService).toasts().at(-1)).toMatchObject({
      variant: 'error',
      message: 're-enter the password when changing the host, port or username',
    })
  })

  describe('a stored password that cannot be decrypted', () => {
    const UNREADABLE = {
      secret_unreadable: true,
      error: "the stored SMTP password cannot be read with this server's SECRETS_ENCRYPTION_KEY",
    }
    const banner = (fixture: { nativeElement: HTMLElement }) =>
      fixture.nativeElement.querySelector('gbt-alert')

    it('warns that the password must be typed again, instead of pretending it is set', () => {
      const { fixture } = render(UNREADABLE)

      expect(fixture.componentInstance.secretUnreadable()).toBe(true)
      expect(fixture.componentInstance.passwordSet()).toBe(false)
      expect(banner(fixture)!.textContent).toContain('illisible')
      expect(banner(fixture)!.textContent).toContain('Saisissez-le à nouveau')
    })

    it('requires the password again before saving', () => {
      const { fixture } = render(UNREADABLE)
      fixture.componentInstance.host.set('smtp.example.com')
      fixture.componentInstance.username.set('relay')
      fixture.componentInstance.fromAddress.set('artiferris@example.com')

      fixture.componentInstance.save()

      expect(fixture.componentInstance.hasErrors()).toBe(true)
    })

    it('drops the warning once the settings are saved with a new password', () => {
      const { fixture, httpMock } = render(UNREADABLE)
      fixture.componentInstance.host.set('smtp.example.com')
      fixture.componentInstance.username.set('relay')
      fixture.componentInstance.password.set('fresh')
      fixture.componentInstance.fromAddress.set('artiferris@example.com')

      fixture.componentInstance.save()
      httpMock
        .expectOne('/api/admin/settings/smtp')
        .flush(null, { status: 204, statusText: 'No Content' })
      fixture.detectChanges()

      expect(fixture.componentInstance.secretUnreadable()).toBe(false)
      expect(banner(fixture)).toBeNull()
    })

    it('shows no warning for readable or absent settings', () => {
      const { fixture } = render(null)

      expect(fixture.componentInstance.secretUnreadable()).toBe(false)
      expect(banner(fixture)).toBeNull()
    })
  })

  it('sends a test email to the given recipient', () => {
    const { fixture, httpMock } = render({
      host: 'smtp.example.com',
      port: 587,
      username: 'artiferris@example.com',
      from_name: 'ArtiFerris',
      from_address: 'artiferris@example.com',
      security: 'start_tls',
      password_set: true,
    })
    fixture.componentInstance.testRecipient.set('admin@example.com')

    fixture.componentInstance.sendTest()

    const req = httpMock.expectOne('/api/admin/settings/smtp/test')
    expect(req.request.method).toBe('POST')
    expect(req.request.body).toEqual({ to: 'admin@example.com' })
    const toastService = TestBed.inject(ToastService)
    req.flush(null)
    fixture.detectChanges()

    expect(toastService.toasts().at(-1)).toMatchObject({
      variant: 'success',
      message: 'E-mail de test envoyé.',
    })
  })

  it('shows an error toast when the test email fails to send', () => {
    const { fixture, httpMock } = render({
      host: 'smtp.example.com',
      port: 587,
      username: 'artiferris@example.com',
      from_name: 'ArtiFerris',
      from_address: 'artiferris@example.com',
      security: 'start_tls',
      password_set: true,
    })
    fixture.componentInstance.testRecipient.set('admin@example.com')

    fixture.componentInstance.sendTest()

    const toastService = TestBed.inject(ToastService)
    httpMock
      .expectOne('/api/admin/settings/smtp/test')
      .flush(null, { status: 500, statusText: 'Internal Server Error' })
    fixture.detectChanges()

    expect(toastService.toasts().at(-1)).toMatchObject({
      variant: 'error',
      message: "Échec de l'envoi de l'e-mail de test. Vérifiez la configuration.",
    })
  })

  it('re-fetches (and resets stale fields) when organizationId changes to a different organization — the component is reused, not recreated, across a super-admin switching organizations', () => {
    const { fixture, httpMock } = render(null)
    fixture.componentRef.setInput('organizationId', 'org-1')
    fixture.detectChanges()
    httpMock.expectOne('/api/admin/settings/smtp?organization_id=org-1').flush({
      host: 'org1.example.com',
      port: 587,
      username: 'org1@example.com',
      from_name: 'Org1',
      from_address: 'org1@example.com',
      security: 'start_tls',
      password_set: true,
    })
    fixture.detectChanges()
    expect(fixture.componentInstance.host()).toBe('org1.example.com')

    fixture.componentRef.setInput('organizationId', 'org-2')
    fixture.detectChanges()
    // org-2 has no SMTP configured — must not still show org-1's settings.
    httpMock.expectOne('/api/admin/settings/smtp?organization_id=org-2').flush(null)
    fixture.detectChanges()

    expect(fixture.componentInstance.host()).toBe('')
    expect(fixture.componentInstance.fromName()).toBe('ArtiFerris')
    expect(fixture.componentInstance.passwordSet()).toBe(false)
  })

  it('explains via a tooltip that the test email uses the saved configuration, not the unsaved form', () => {
    const { fixture } = render(null)

    const tooltip = fixture.debugElement.query(By.directive(Tooltip))

    expect((tooltip.componentInstance as Tooltip).text()).toBe(
      'Utilise la configuration déjà enregistrée, pas les modifications du formulaire ci-dessus.',
    )
  })

  describe('switching organizations', () => {
    const smtp = (host: string) => ({
      host,
      port: 587,
      username: 'u',
      from_name: 'n',
      from_address: 'a@b.c',
      security: 'start_tls',
      password_set: true,
    })

    function switchTwice() {
      const { fixture, httpMock } = render(null)
      fixture.componentRef.setInput('organizationId', 'org-a')
      fixture.detectChanges()
      const forA = httpMock.expectOne('/api/admin/settings/smtp?organization_id=org-a')
      fixture.componentRef.setInput('organizationId', 'org-b')
      fixture.detectChanges()
      const forB = httpMock.expectOne('/api/admin/settings/smtp?organization_id=org-b')
      return { fixture, forA, forB }
    }

    it('keeps the current organization when a slower response for the previous one lands last', () => {
      const { fixture, forA, forB } = switchTwice()

      forB.flush(smtp('b.example.com'))
      forA.flush(smtp('a.example.com'))
      fixture.detectChanges()

      expect(fixture.componentInstance.host()).toBe('b.example.com')
    })

    it('clears a password typed for the previous organization, so saving does not send it to the next one', () => {
      const { fixture, httpMock } = render(null)
      fixture.componentRef.setInput('organizationId', 'org-a')
      fixture.detectChanges()
      httpMock
        .expectOne('/api/admin/settings/smtp?organization_id=org-a')
        .flush(smtp('a.example.com'))
      fixture.componentInstance.password.set('secret-of-a')
      fixture.componentInstance.testRecipient.set('a@example.com')
      fixture.componentInstance.save()
      httpMock.expectOne('/api/admin/settings/smtp?organization_id=org-a').flush(null)
      fixture.componentInstance.password.set('typed-again-for-a')

      fixture.componentRef.setInput('organizationId', 'org-b')
      fixture.detectChanges()
      httpMock
        .expectOne('/api/admin/settings/smtp?organization_id=org-b')
        .flush(smtp('b.example.com'))
      fixture.detectChanges()

      expect(fixture.componentInstance.password()).toBe('')
      expect(fixture.componentInstance.testRecipient()).toBe('')
      expect(fixture.componentInstance.attemptedSave()).toBe(false)

      fixture.componentInstance.save()
      const put = httpMock.expectOne('/api/admin/settings/smtp?organization_id=org-b')
      expect(put.request.method).toBe('PUT')
      expect(put.request.body.password).toBeUndefined()
      put.flush(null)
    })

    it('does not mark the next organization as configured when a save for the previous one lands late', () => {
      const { fixture, httpMock } = render(null)
      fixture.componentRef.setInput('organizationId', 'org-a')
      fixture.detectChanges()
      httpMock.expectOne('/api/admin/settings/smtp?organization_id=org-a').flush(null)
      fixture.componentInstance.host.set('a.example.com')
      fixture.componentInstance.username.set('u')
      fixture.componentInstance.password.set('pw')
      fixture.componentInstance.fromAddress.set('a@b.c')
      fixture.componentInstance.save()
      const lateSave = httpMock.expectOne('/api/admin/settings/smtp?organization_id=org-a')

      fixture.componentRef.setInput('organizationId', 'org-b')
      fixture.detectChanges()
      httpMock.expectOne('/api/admin/settings/smtp?organization_id=org-b').flush(null)
      lateSave.flush(null)

      expect(fixture.componentInstance.passwordSet()).toBe(false)
    })

    it('shows an error, not an endless spinner, when the load fails', () => {
      const { fixture, forA, forB } = switchTwice()

      forA.flush(null, { status: 500, statusText: 'boom' })
      forB.flush(null, { status: 500, statusText: 'boom' })
      fixture.detectChanges()

      expect(fixture.nativeElement.querySelector('gbt-spinner')).toBeNull()
      expect(fixture.nativeElement.textContent).toContain(
        'Échec du chargement des paramètres SMTP.',
      )
    })
  })
})
