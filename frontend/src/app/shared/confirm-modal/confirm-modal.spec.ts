import { TestBed } from '@angular/core/testing'
import { ConfirmModal } from './confirm-modal'

describe('ConfirmModal', () => {
  function render(
    props: { heading?: string; message?: string; confirmLabel?: string; confirming?: boolean } = {},
  ) {
    TestBed.configureTestingModule({
      imports: [ConfirmModal],
    })
    const fixture = TestBed.createComponent(ConfirmModal)
    fixture.componentRef.setInput('heading', props.heading ?? 'Test Heading')
    fixture.componentRef.setInput('message', props.message ?? 'Test Message')
    if (props.confirmLabel !== undefined) {
      fixture.componentRef.setInput('confirmLabel', props.confirmLabel)
    }
    if (props.confirming !== undefined) {
      fixture.componentRef.setInput('confirming', props.confirming)
    }
    fixture.detectChanges()
    return fixture
  }

  it('emits confirmed when the confirm button is clicked', () => {
    const fixture = render()
    let confirmed = false
    fixture.componentInstance.confirmed.subscribe(() => (confirmed = true))

    const button = fixture.nativeElement.querySelector('gbt-button')
    button.dispatchEvent(new CustomEvent('clicked'))

    expect(confirmed).toBe(true)
  })

  it('emits cancelled when the modal is closed', () => {
    const fixture = render()
    let cancelled = false
    fixture.componentInstance.cancelled.subscribe(() => (cancelled = true))

    const modal = fixture.nativeElement.querySelector('gbt-modal')
    modal.dispatchEvent(new CustomEvent('closed'))

    expect(cancelled).toBe(true)
  })

  it('receives the heading input', () => {
    const fixture = render({ heading: 'Delete Item?' })

    expect(fixture.componentInstance.heading()).toBe('Delete Item?')
  })

  it('receives the message input', () => {
    const fixture = render({ message: 'This action cannot be undone.' })

    expect(fixture.componentInstance.message()).toBe('This action cannot be undone.')
  })

  it('uses the default confirm label "Confirmer" when not provided', () => {
    const fixture = render()

    expect(fixture.componentInstance.confirmLabel()).toBe('Confirmer')
  })

  it('uses a custom confirm label when provided', () => {
    const fixture = render({ confirmLabel: 'Supprimer' })

    expect(fixture.componentInstance.confirmLabel()).toBe('Supprimer')
  })

  it('reflects the confirming loading state', () => {
    const fixture = render({ confirming: false })
    expect(fixture.componentInstance.confirming()).toBe(false)

    fixture.componentRef.setInput('confirming', true)
    fixture.detectChanges()
    expect(fixture.componentInstance.confirming()).toBe(true)
  })
})
