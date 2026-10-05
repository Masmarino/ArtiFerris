import { TestBed } from '@angular/core/testing'
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing'
import { provideHttpClient } from '@angular/common/http'
import { CreateUserModal } from './create-user-modal'
import { userProviders } from '../infrastructure/user.providers'

describe('CreateUserModal', () => {
  let httpMock: HttpTestingController

  beforeEach(() => {
    TestBed.configureTestingModule({
      imports: [CreateUserModal],
      providers: [provideHttpClient(), provideHttpClientTesting(), ...userProviders],
    })
    httpMock = TestBed.inject(HttpTestingController)
  })

  afterEach(() => {
    httpMock.verify()
  })

  function open() {
    const fixture = TestBed.createComponent(CreateUserModal)
    fixture.detectChanges()
    return { fixture, modal: fixture.componentInstance }
  }

  it('posts the address and is_super_admin: true when the switch is on', () => {
    const { modal } = open()

    modal.onEmailChange('florian@example.com')
    modal.isSuperAdmin.set(true)
    modal.submit()

    const request = httpMock.expectOne('/api/users')
    expect(request.request.method).toBe('POST')
    expect(request.request.body).toEqual({ email: 'florian@example.com', is_super_admin: true })
    request.flush({})
  })

  it('sends nothing and says what to type when the address is missing or malformed', () => {
    const { modal } = open()

    modal.submit()
    expect(modal.emailError()).toBe("Saisissez l'adresse e-mail")

    modal.onEmailChange('florian@example')
    modal.submit()
    expect(modal.emailError()).toBe(
      'Saisissez une adresse e-mail valide, par exemple nom@exemple.fr',
    )
    httpMock.expectNone('/api/users')
  })

  it('clears the address error as soon as the address changes', () => {
    const { modal } = open()

    modal.submit()
    modal.onEmailChange('f')

    expect(modal.emailError()).toBeNull()
  })

  it('shows the outcome in the dialog and tells the parent', () => {
    const { fixture, modal } = open()
    const invited = vi.fn()
    modal.invited.subscribe(invited)

    modal.onEmailChange('  alice@example.com ')
    modal.submit()
    httpMock.expectOne('/api/users').flush({ id: 'u9', email_sent: true })
    fixture.detectChanges()

    expect(invited).toHaveBeenCalledWith({ id: 'u9', email_sent: true })
    expect(document.body.textContent).toContain('Invitation envoyée à alice@example.com')
  })

  it('hands over the activation link when the mail could not go out', () => {
    const { fixture, modal } = open()

    modal.onEmailChange('alice@example.com')
    modal.submit()
    httpMock.expectOne('/api/users').flush({
      id: 'u9',
      email_sent: false,
      email_error: 'email_not_configured',
      activation_url: 'https://app.example.com/activate#token=abc',
    })
    fixture.detectChanges()

    const text = document.body.textContent ?? ''
    expect(text).toContain("Le mail n'a pas pu être envoyé")
    expect(text).toContain("Aucun serveur mail n'est configuré pour cette organisation.")
    expect(text).toContain('Transmettez ce lien à alice@example.com')
    expect(text).not.toContain('Invitation envoyée')
    expect(document.body.querySelector('gbt-copy-field code')?.textContent).toBe(
      'https://app.example.com/activate#token=abc',
    )
  })

  it('puts a taken address under the field and keeps the draft', () => {
    const { modal } = open()

    modal.onEmailChange('alice@example.com')
    modal.submit()
    httpMock
      .expectOne('/api/users')
      .flush(
        { error: 'email already in use', code: 'email_taken' },
        { status: 409, statusText: 'Conflict' },
      )

    expect(modal.emailError()).toBe('Cette adresse e-mail est déjà utilisée par un compte')
    expect(modal.email()).toBe('alice@example.com')
    expect(modal.result()).toBeNull()
  })

  it('says the invitation could not be sent on any other failure', () => {
    const { modal } = open()

    modal.onEmailChange('alice@example.com')
    modal.submit()
    httpMock.expectOne('/api/users').flush('boom', { status: 500, statusText: 'Server Error' })

    expect(modal.formError()).toBe("L'invitation n'a pas pu être envoyée. Réessayez plus tard.")
    expect(modal.sending()).toBe(false)
  })

  it('ignores a second submit while the first is on its way', () => {
    const { modal } = open()

    modal.onEmailChange('alice@example.com')
    modal.submit()
    modal.submit()

    httpMock.expectOne('/api/users').flush({})
  })
})
