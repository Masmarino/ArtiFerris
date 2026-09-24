import { TestBed } from '@angular/core/testing'
import { CopyableCommand } from './copyable-command'

function render() {
  const fixture = TestBed.createComponent(CopyableCommand)
  fixture.componentRef.setInput('command', 'npm install left-pad')
  fixture.componentRef.setInput('label', 'Copier la commande')
  fixture.detectChanges()
  const el: HTMLElement = fixture.nativeElement
  return { fixture, el, button: el.querySelector<HTMLButtonElement>('gbt-button button')! }
}

describe('CopyableCommand', () => {
  afterEach(() => {
    vi.useRealTimers()
    vi.unstubAllGlobals()
  })

  it('shows the command and names the copy button', () => {
    const { el, button } = render()

    expect(el.querySelector('pre')!.textContent).toBe('npm install left-pad')
    expect(button.getAttribute('aria-label')).toBe('Copier la commande')
  })

  it('copies the command and announces it', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined)
    vi.stubGlobal('navigator', { clipboard: { writeText } })
    const { fixture, el, button } = render()

    button.click()
    await fixture.whenStable()
    fixture.detectChanges()

    expect(writeText).toHaveBeenCalledWith('npm install left-pad')
    expect(el.querySelector('[aria-live="polite"]')!.textContent).toBe('Commande copiée')
  })

  it('reports a clipboard failure', async () => {
    vi.stubGlobal('navigator', {
      clipboard: { writeText: vi.fn().mockRejectedValue(new Error('denied')) },
    })
    const { fixture, el, button } = render()

    button.click()
    await fixture.whenStable()
    fixture.detectChanges()

    expect(el.querySelector('[aria-live="polite"]')!.textContent).toContain('Copie impossible')
  })

  it('goes back to idle after two seconds', async () => {
    vi.stubGlobal('navigator', { clipboard: { writeText: vi.fn().mockResolvedValue(undefined) } })
    vi.useFakeTimers()
    const { fixture, el, button } = render()

    button.click()
    await vi.advanceTimersByTimeAsync(0)
    fixture.detectChanges()
    expect(el.querySelector('[aria-live="polite"]')!.textContent).toBe('Commande copiée')

    await vi.advanceTimersByTimeAsync(2000)
    fixture.detectChanges()

    expect(el.querySelector('[aria-live="polite"]')!.textContent).toBe('')
  })
})
