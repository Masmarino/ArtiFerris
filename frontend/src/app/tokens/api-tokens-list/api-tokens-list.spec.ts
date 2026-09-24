import { TestBed } from '@angular/core/testing'
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing'
import { provideHttpClient } from '@angular/common/http'
import { By } from '@angular/platform-browser'
import { Table } from '@masmarino/gabarit'
import { ApiTokensList } from './api-tokens-list'
import { apiTokenProviders } from '../infrastructure/api-token.providers'
import { ConfirmService } from '../../shared/confirm.service'

describe('ApiTokensList', () => {
  it('loads and displays tokens on init', () => {
    TestBed.configureTestingModule({
      providers: [provideHttpClient(), provideHttpClientTesting(), ...apiTokenProviders],
    })
    const fixture = TestBed.createComponent(ApiTokensList)
    const httpMock = TestBed.inject(HttpTestingController)

    fixture.detectChanges()
    httpMock
      .expectOne('/api/tokens')
      .flush([{ id: '1', label: 'laptop', created_at: '2026-01-01T00:00:00Z', last_used_at: null }])
    fixture.detectChanges()

    expect(fixture.componentInstance.tokens().length).toBe(1)
  })

  it('shows the plaintext token exactly once after creating it, via a real button click', () => {
    TestBed.configureTestingModule({
      providers: [provideHttpClient(), provideHttpClientTesting(), ...apiTokenProviders],
    })
    const fixture = TestBed.createComponent(ApiTokensList)
    const httpMock = TestBed.inject(HttpTestingController)

    fixture.detectChanges()
    httpMock.expectOne('/api/tokens').flush([])
    fixture.detectChanges()

    fixture.componentInstance.showCreateForm.set(true)
    fixture.componentInstance.newLabel.setValue('laptop')
    fixture.detectChanges()

    fixture.componentInstance.createToken()
    httpMock.expectOne('/api/tokens').flush({ id: '2', token: 'hgr_plaintext-value' })
    httpMock
      .expectOne('/api/tokens')
      .flush([{ id: '2', label: 'laptop', created_at: '2026-01-01T00:00:00Z', last_used_at: null }])
    fixture.detectChanges()

    expect(fixture.componentInstance.createdToken()).toBe('hgr_plaintext-value')
    expect(fixture.nativeElement.textContent).toContain('hgr_plaintext-value')
  })

  it('revokes a token when a table row is clicked, after confirmation', async () => {
    const ask = vi.fn().mockResolvedValue(true)
    TestBed.configureTestingModule({
      providers: [
        provideHttpClient(),
        provideHttpClientTesting(),
        ...apiTokenProviders,
        { provide: ConfirmService, useValue: { ask } },
      ],
    })
    const fixture = TestBed.createComponent(ApiTokensList)
    const httpMock = TestBed.inject(HttpTestingController)

    fixture.detectChanges()
    httpMock
      .expectOne('/api/tokens')
      .flush([{ id: '1', label: 'laptop', created_at: '2026-01-01T00:00:00Z', last_used_at: null }])
    fixture.detectChanges()

    const table = fixture.debugElement.query(By.directive(Table))
    table.triggerEventHandler('rowClick', {
      id: '1',
      label: 'laptop',
      created_at: '2026-01-01T00:00:00Z',
      last_used_at: null,
    })

    await fixture.whenStable()

    expect(ask).toHaveBeenCalledWith(
      expect.objectContaining({ heading: 'Révoquer le token', danger: true }),
    )
    httpMock.expectOne('/api/tokens/1').flush(null)
    httpMock.expectOne('/api/tokens').flush([])
  })

  it('does not revoke the token when the confirmation is declined', async () => {
    const ask = vi.fn().mockResolvedValue(false)
    TestBed.configureTestingModule({
      providers: [
        provideHttpClient(),
        provideHttpClientTesting(),
        ...apiTokenProviders,
        { provide: ConfirmService, useValue: { ask } },
      ],
    })
    const fixture = TestBed.createComponent(ApiTokensList)
    const httpMock = TestBed.inject(HttpTestingController)

    fixture.detectChanges()
    httpMock.expectOne('/api/tokens').flush([])

    await fixture.componentInstance.revokeToken({
      id: '1',
      label: 'laptop',
      created_at: '2026-01-01T00:00:00Z',
      last_used_at: null,
    })

    expect(ask).toHaveBeenCalled()
    httpMock.expectNone('/api/tokens/1')
  })

  describe('token lifetime', () => {
    function open() {
      TestBed.configureTestingModule({
        providers: [provideHttpClient(), provideHttpClientTesting(), ...apiTokenProviders],
      })
      const fixture = TestBed.createComponent(ApiTokensList)
      const httpMock = TestBed.inject(HttpTestingController)
      fixture.detectChanges()
      httpMock.expectOne('/api/tokens').flush([])
      fixture.componentInstance.showCreateForm.set(true)
      fixture.componentInstance.newLabel.setValue('laptop')
      fixture.detectChanges()
      return { fixture, httpMock }
    }
    const hint = (fixture: { nativeElement: HTMLElement }) =>
      fixture.nativeElement.querySelector('.api-tokens-list__hint')!.textContent!.trim()

    it('offers an optional password field and announces 7 days without it', () => {
      const { fixture } = open()

      expect(fixture.nativeElement.textContent).toContain(
        'Mot de passe (pour un jeton de longue durée)',
      )
      expect(hint(fixture)).toContain('7 jours')
    })

    it('announces 365 days once a password is typed', () => {
      const { fixture } = open()

      fixture.componentInstance.newPassword.setValue('s3cret!')
      fixture.detectChanges()

      expect(hint(fixture)).toBe('Ce token expirera dans 365 jours.')
    })

    it('sends no current_password when the field is empty, and reports 7 days on the created token', () => {
      const { fixture, httpMock } = open()

      fixture.componentInstance.createToken()
      const req = httpMock.expectOne('/api/tokens')
      expect(req.request.body).toEqual({ label: 'laptop' })
      req.flush({ id: '2', token: 'hgr_plain' })
      httpMock.expectOne('/api/tokens').flush([])
      fixture.detectChanges()

      expect(hint(fixture)).toBe('Il expirera dans 7 jours.')
    })

    it('sends the password and reports 365 days on the created token, then forgets the password', () => {
      const { fixture, httpMock } = open()
      fixture.componentInstance.newPassword.setValue('s3cret!')

      fixture.componentInstance.createToken()
      const req = httpMock.expectOne('/api/tokens')
      expect(req.request.body).toEqual({ label: 'laptop', current_password: 's3cret!' })
      req.flush({ id: '2', token: 'hgr_plain' })
      httpMock.expectOne('/api/tokens').flush([])
      fixture.detectChanges()

      expect(hint(fixture)).toBe('Il expirera dans 365 jours.')
      expect(fixture.componentInstance.newPassword.value).toBe('')
    })

    it('keeps the form open with an error when the password is wrong, and clears the password', () => {
      const { fixture, httpMock } = open()
      fixture.componentInstance.newPassword.setValue('wrong')

      fixture.componentInstance.createToken()
      httpMock
        .expectOne('/api/tokens')
        .flush({ error: 'invalid credentials' }, { status: 400, statusText: 'Bad Request' })
      fixture.detectChanges()

      expect(fixture.nativeElement.querySelector('[role="alert"]').textContent).toContain(
        'Mot de passe incorrect.',
      )
      expect(fixture.componentInstance.showCreateForm()).toBe(true)
      expect(fixture.componentInstance.newPassword.value).toBe('')
    })

    it('reports throttling distinctly from a wrong password', () => {
      const { fixture, httpMock } = open()
      fixture.componentInstance.newPassword.setValue('wrong')

      fixture.componentInstance.createToken()
      httpMock
        .expectOne('/api/tokens')
        .flush(null, { status: 429, statusText: 'Too Many Requests' })
      fixture.detectChanges()

      expect(fixture.nativeElement.querySelector('[role="alert"]').textContent).toContain(
        'Trop de tentatives',
      )
    })

    it('shows the server message for a 400 that is not about the password', () => {
      const { fixture, httpMock } = open()
      fixture.componentInstance.newPassword.setValue('s3cret!')

      fixture.componentInstance.createToken()
      httpMock
        .expectOne('/api/tokens')
        .flush({ error: 'token label is too long' }, { status: 400, statusText: 'Bad Request' })
      fixture.detectChanges()

      const alert = fixture.nativeElement.querySelector('[role="alert"]').textContent
      expect(alert).toContain('token label is too long')
      expect(alert).not.toContain('Mot de passe incorrect.')
    })

    it('reports a wrong password on a 403', () => {
      const { fixture, httpMock } = open()
      fixture.componentInstance.newPassword.setValue('wrong')

      fixture.componentInstance.createToken()
      httpMock.expectOne('/api/tokens').flush(null, { status: 403, statusText: 'Forbidden' })
      fixture.detectChanges()

      expect(fixture.nativeElement.querySelector('[role="alert"]').textContent).toContain(
        'Mot de passe incorrect.',
      )
    })

    it('flags a label longer than the server accepts', () => {
      const { fixture } = open()

      fixture.componentInstance.newLabel.setValue('x'.repeat(101))
      fixture.detectChanges()

      expect(fixture.componentInstance.newLabel.invalid).toBe(true)
      expect(fixture.nativeElement.textContent).toContain('dépasser 100 caractères')
    })

    it('shows a generic error for another failure', () => {
      const { fixture, httpMock } = open()

      fixture.componentInstance.createToken()
      httpMock.expectOne('/api/tokens').flush(null, { status: 500, statusText: 'Server Error' })
      fixture.detectChanges()

      expect(fixture.nativeElement.querySelector('[role="alert"]').textContent).toContain(
        'Échec de la création du token.',
      )
    })
  })
})
