import { TestBed } from '@angular/core/testing'
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing'
import { provideHttpClient } from '@angular/common/http'
import { ApiTokensList } from './api-tokens-list'
import { apiTokenProviders } from '../infrastructure/api-token.providers'
import { ConfirmService } from '../../shared/confirm.service'

const LAPTOP = { id: '1', label: 'laptop', created_at: '2026-01-01T00:00:00Z', last_used_at: null }

function setup(confirm?: { ask: (options: unknown) => Promise<boolean> }) {
  TestBed.configureTestingModule({
    providers: [
      provideHttpClient(),
      provideHttpClientTesting(),
      ...apiTokenProviders,
      ...(confirm ? [{ provide: ConfirmService, useValue: confirm }] : []),
    ],
  })
  const fixture = TestBed.createComponent(ApiTokensList)
  const httpMock = TestBed.inject(HttpTestingController)
  fixture.detectChanges()
  return { fixture, httpMock, el: fixture.nativeElement as HTMLElement }
}

const text = (el: Element | null | undefined) => el?.textContent?.replace(/\s+/g, ' ').trim() ?? ''
const card = (el: HTMLElement, heading: string) =>
  Array.from(el.querySelectorAll('gbt-card')).find((each) =>
    text(each.querySelector('h2, h3')).startsWith(heading),
  )

describe('ApiTokensList', () => {
  it('lists the active tokens as rows, with when each was created and last used', () => {
    const { fixture, httpMock, el } = setup()

    httpMock
      .expectOne('/api/tokens')
      .flush([LAPTOP, { ...LAPTOP, id: '2', label: 'ci', last_used_at: '2026-01-02T00:00:00Z' }])
    fixture.detectChanges()

    const rows = el.querySelectorAll('.api-tokens-list__rows > li')
    expect(rows.length).toBe(2)
    expect(text(rows[0])).toContain('laptop')
    expect(text(rows[0])).toContain('Créé le')
    expect(text(rows[0])).toContain('Jamais utilisé')
    expect(text(rows[1])).toContain('Utilisé le')
    expect(text(card(el, 'Jetons actifs')?.querySelector('.gbt-card__count'))).toBe('2')
  })

  it('says there are none yet, in the list card', () => {
    const { fixture, httpMock, el } = setup()

    httpMock.expectOne('/api/tokens').flush([])
    fixture.detectChanges()

    expect(text(card(el, 'Jetons actifs'))).toContain('Aucun jeton')
  })

  it('says so when the list cannot be loaded, and loads it again on demand', () => {
    const { fixture, httpMock, el } = setup()

    httpMock.expectOne('/api/tokens').flush(null, { status: 500, statusText: 'Server Error' })
    fixture.detectChanges()
    const list = card(el, 'Jetons actifs')!
    expect(text(list.querySelector('gbt-alert'))).toContain("Les jetons n'ont pas pu être chargés.")

    list.querySelector<HTMLButtonElement>('gbt-alert button')!.click()
    httpMock.expectOne('/api/tokens').flush([LAPTOP])
    fixture.detectChanges()
    expect(card(el, 'Jetons actifs')!.querySelector('gbt-alert')).toBeNull()
    expect(el.querySelectorAll('.api-tokens-list__rows > li').length).toBe(1)
  })

  it('shows the new token once, in a card of its own, and marks its row as new', () => {
    const { fixture, httpMock, el } = setup()
    httpMock.expectOne('/api/tokens').flush([])
    fixture.componentInstance.newLabel.setValue('laptop')
    fixture.detectChanges()

    el.querySelector<HTMLFormElement>('.api-tokens-list__form')!.dispatchEvent(new Event('submit'))
    httpMock.expectOne('/api/tokens').flush({ id: '1', token: 'hgr_plaintext-value' })
    httpMock.expectOne('/api/tokens').flush([LAPTOP])
    fixture.detectChanges()

    const revealed = el.querySelector('.api-tokens-list__revealed')!
    expect(text(revealed)).toContain('Jeton « laptop » généré')
    expect(text(revealed)).toContain('hgr_plaintext-value')
    expect(text(el.querySelector('.api-tokens-list__rows'))).toContain('Nouveau')
    expect(fixture.componentInstance.newLabel.value).toBe('')

    revealed.querySelector<HTMLButtonElement>('.api-tokens-list__reveal-footer button')!.click()
    fixture.detectChanges()
    expect(el.textContent).not.toContain('hgr_plaintext-value')
  })

  it('cannot generate a token without a name', () => {
    const { fixture, httpMock, el } = setup()
    httpMock.expectOne('/api/tokens').flush([])
    fixture.detectChanges()

    expect(el.querySelector<HTMLButtonElement>('.api-tokens-list__submit button')!.disabled).toBe(
      true,
    )
    fixture.componentInstance.createToken()
    httpMock.expectNone((req) => req.method === 'POST')
  })

  it('revokes a token from its row, after confirmation', async () => {
    const ask = vi.fn().mockResolvedValue(true)
    const { fixture, httpMock, el } = setup({ ask })
    httpMock.expectOne('/api/tokens').flush([LAPTOP])
    fixture.detectChanges()

    const revoke = el.querySelector<HTMLButtonElement>('.api-tokens-list__revoke button')!
    expect(revoke.getAttribute('aria-label')).toBe('Révoquer le jeton laptop')
    revoke.click()
    await fixture.whenStable()

    expect(ask).toHaveBeenCalledWith(
      expect.objectContaining({ heading: 'Révoquer le jeton', danger: true }),
    )
    httpMock.expectOne('/api/tokens/1').flush(null)
    httpMock.expectOne('/api/tokens').flush([])
  })

  it('does not revoke the token when the confirmation is declined', async () => {
    const ask = vi.fn().mockResolvedValue(false)
    const { fixture, httpMock } = setup({ ask })
    httpMock.expectOne('/api/tokens').flush([])

    await fixture.componentInstance.revokeToken(LAPTOP)

    expect(ask).toHaveBeenCalled()
    httpMock.expectNone('/api/tokens/1')
  })

  describe('token lifetime', () => {
    function open() {
      const context = setup()
      context.httpMock.expectOne('/api/tokens').flush([])
      context.fixture.componentInstance.newLabel.setValue('laptop')
      context.fixture.detectChanges()
      return context
    }
    const hint = (el: HTMLElement) => text(el.querySelector('.api-tokens-list__hint'))
    const createError = (el: HTMLElement) =>
      text(el.querySelector('.api-tokens-list__form gbt-alert'))

    it('offers an optional password field and announces 7 days without it', () => {
      const { el } = open()

      expect(el.textContent).toContain('Mot de passe (facultatif)')
      expect(hint(el)).toContain('7 jours')
    })

    it('announces 365 days once a password is typed', () => {
      const { fixture, el } = open()

      fixture.componentInstance.newPassword.setValue('s3cret!')
      fixture.detectChanges()

      expect(hint(el)).toBe('Ce jeton expirera dans 365 jours.')
    })

    it('sends no current_password when the field is empty, and says the token lasts 7 days', () => {
      const { fixture, httpMock, el } = open()

      fixture.componentInstance.createToken()
      const req = httpMock.expectOne('/api/tokens')
      expect(req.request.body).toEqual({ label: 'laptop' })
      req.flush({ id: '2', token: 'hgr_plain' })
      httpMock.expectOne('/api/tokens').flush([])
      fixture.detectChanges()

      expect(text(el.querySelector('.api-tokens-list__revealed'))).toContain(
        'Il expirera dans 7 jours.',
      )
    })

    it('sends the password, says the token lasts 365 days, then forgets the password', () => {
      const { fixture, httpMock, el } = open()
      fixture.componentInstance.newPassword.setValue('s3cret!')

      fixture.componentInstance.createToken()
      const req = httpMock.expectOne('/api/tokens')
      expect(req.request.body).toEqual({ label: 'laptop', current_password: 's3cret!' })
      req.flush({ id: '2', token: 'hgr_plain' })
      httpMock.expectOne('/api/tokens').flush([])
      fixture.detectChanges()

      expect(text(el.querySelector('.api-tokens-list__revealed'))).toContain(
        'Il expirera dans 365 jours.',
      )
      expect(fixture.componentInstance.newPassword.value).toBe('')
    })

    it('keeps the name with an error when the password is wrong, and clears the password', () => {
      const { fixture, httpMock, el } = open()
      fixture.componentInstance.newPassword.setValue('wrong')

      fixture.componentInstance.createToken()
      httpMock
        .expectOne('/api/tokens')
        .flush(
          { error: 'invalid credentials', code: 'invalid_credentials' },
          { status: 400, statusText: 'Bad Request' },
        )
      fixture.detectChanges()

      expect(createError(el)).toContain('Mot de passe incorrect.')
      expect(fixture.componentInstance.newLabel.value).toBe('laptop')
      expect(fixture.componentInstance.newPassword.value).toBe('')
    })

    it('reports throttling distinctly from a wrong password', () => {
      const { fixture, httpMock, el } = open()
      fixture.componentInstance.newPassword.setValue('wrong')

      fixture.componentInstance.createToken()
      httpMock
        .expectOne('/api/tokens')
        .flush(null, { status: 429, statusText: 'Too Many Requests' })
      fixture.detectChanges()

      expect(createError(el)).toContain('Trop de tentatives')
    })

    it('shows the server message for a 400 that is not about the password', () => {
      const { fixture, httpMock, el } = open()
      fixture.componentInstance.newPassword.setValue('s3cret!')

      fixture.componentInstance.createToken()
      httpMock
        .expectOne('/api/tokens')
        .flush({ error: 'token label is too long' }, { status: 400, statusText: 'Bad Request' })
      fixture.detectChanges()

      expect(createError(el)).toContain('token label is too long')
      expect(createError(el)).not.toContain('Mot de passe incorrect.')
    })

    it('reports a wrong password on a 403', () => {
      const { fixture, httpMock, el } = open()
      fixture.componentInstance.newPassword.setValue('wrong')

      fixture.componentInstance.createToken()
      httpMock.expectOne('/api/tokens').flush(null, { status: 403, statusText: 'Forbidden' })
      fixture.detectChanges()

      expect(createError(el)).toContain('Mot de passe incorrect.')
    })

    it('flags a name longer than the server accepts, and sends nothing', () => {
      const { fixture, httpMock, el } = open()

      fixture.componentInstance.newLabel.setValue('x'.repeat(101))
      fixture.detectChanges()
      fixture.componentInstance.createToken()

      expect(fixture.componentInstance.newLabel.invalid).toBe(true)
      expect(el.textContent).toContain('dépasser 100 caractères')
      httpMock.expectNone((req) => req.method === 'POST')
    })

    it('shows a generic error for another failure', () => {
      const { fixture, httpMock, el } = open()

      fixture.componentInstance.createToken()
      httpMock.expectOne('/api/tokens').flush(null, { status: 500, statusText: 'Server Error' })
      fixture.detectChanges()

      expect(createError(el)).toContain('Impossible de générer le jeton.')
    })
  })
})
