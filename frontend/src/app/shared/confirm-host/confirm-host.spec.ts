import { TestBed } from '@angular/core/testing'
import { By } from '@angular/platform-browser'
import { Button } from '@masmarino/gabarit'
import { ConfirmService, type ConfirmOptions } from '../confirm.service'
import { ConfirmHost } from './confirm-host'

describe('ConfirmHost', () => {
  function render() {
    TestBed.configureTestingModule({ imports: [ConfirmHost] })
    const fixture = TestBed.createComponent(ConfirmHost)
    fixture.detectChanges()
    return { fixture, confirm: TestBed.inject(ConfirmService) }
  }

  function open(
    { fixture, confirm }: ReturnType<typeof render>,
    options: Partial<ConfirmOptions> = {},
  ) {
    const answer = confirm.ask({ heading: 'Supprimer', message: 'Sûr ?', ...options })
    fixture.detectChanges()
    return answer
  }

  function buttons(fixture: ReturnType<typeof render>['fixture']) {
    return fixture.debugElement
      .queryAll(By.directive(Button))
      .map((b) => b.componentInstance as Button)
  }

  it('renders nothing while no confirmation is pending', () => {
    const { fixture } = render()

    expect(fixture.nativeElement.querySelector('gbt-modal')).toBeNull()
    expect(fixture.nativeElement.querySelector('gbt-confirm-danger-modal')).toBeNull()
  })

  it('shows a standard dialog with French default labels', () => {
    const view = render()
    void open(view, { heading: 'Révoquer le jeton', message: 'Le jeton cessera de fonctionner.' })

    const modal = view.fixture.nativeElement.querySelector('gbt-modal')
    expect(modal).not.toBeNull()
    expect(modal.textContent).toContain('Le jeton cessera de fonctionner.')
    expect(buttons(view.fixture).map((b) => b.text())).toEqual(['Annuler', 'Confirmer'])
  })

  it('uses the custom labels when given', () => {
    const view = render()
    void open(view, { confirmLabel: 'Révoquer', cancelLabel: 'Garder' })

    expect(buttons(view.fixture).map((b) => b.text())).toEqual(['Garder', 'Révoquer'])
  })

  it('resolves true when the confirm button is clicked, and closes', async () => {
    const view = render()
    const answer = open(view)

    view.fixture.nativeElement
      .querySelectorAll('gbt-button')[1]
      .dispatchEvent(new CustomEvent('clicked'))
    view.fixture.detectChanges()

    expect(await answer).toBe(true)
    expect(view.fixture.nativeElement.querySelector('gbt-modal')).toBeNull()
  })

  it('resolves false when the cancel button is clicked', async () => {
    const view = render()
    const answer = open(view)

    view.fixture.nativeElement
      .querySelectorAll('gbt-button')[0]
      .dispatchEvent(new CustomEvent('clicked'))

    expect(await answer).toBe(false)
  })

  it('resolves false when the dialog is dismissed', async () => {
    const view = render()
    const answer = open(view)

    view.fixture.nativeElement.querySelector('gbt-modal').dispatchEvent(new CustomEvent('closed'))

    expect(await answer).toBe(false)
  })

  it('styles the confirm button as danger only when asked to', () => {
    const plain = render()
    void open(plain)
    expect(buttons(plain.fixture)[1].variant()).toBe('primary')

    TestBed.resetTestingModule()
    const danger = render()
    void open(danger, { danger: true })
    expect(buttons(danger.fixture)[1].variant()).toBe('danger')
  })

  it('switches to the type-to-confirm dialog when a text to retype is given', () => {
    const view = render()
    void open(view, { typeToConfirm: 'acme-web' })

    const dialog = view.fixture.nativeElement.querySelector('gbt-confirm-danger-modal')
    expect(dialog).not.toBeNull()
    expect(view.fixture.nativeElement.querySelector('gbt-modal > p')).toBeNull()
  })

  it('resolves true when the type-to-confirm dialog is confirmed', async () => {
    const view = render()
    const answer = open(view, { typeToConfirm: 'acme-web' })

    view.fixture.nativeElement
      .querySelector('gbt-confirm-danger-modal')
      .dispatchEvent(new CustomEvent('confirmed'))

    expect(await answer).toBe(true)
  })

  it('resolves false when the type-to-confirm dialog is closed', async () => {
    const view = render()
    const answer = open(view, { typeToConfirm: 'acme-web' })

    view.fixture.nativeElement
      .querySelector('gbt-confirm-danger-modal')
      .dispatchEvent(new CustomEvent('closed'))

    expect(await answer).toBe(false)
  })
})
