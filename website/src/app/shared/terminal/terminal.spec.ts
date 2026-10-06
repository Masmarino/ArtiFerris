import { Component } from '@angular/core'
import { TestBed } from '@angular/core/testing'
import { Terminal } from './terminal'

@Component({
  imports: [Terminal],
  template: `<app-terminal [text]="text" label="Output" [wrap]="true">The caption</app-terminal>`,
})
class Host {
  text = '$ echo one\none\ntwo'
}

describe('Terminal', () => {
  async function render() {
    const fixture = TestBed.createComponent(Host)
    await fixture.whenStable()
    return fixture.nativeElement as HTMLElement
  }

  it('prints the output as written, named and reachable with the keyboard', async () => {
    const root = await render()
    const pre = root.querySelector('pre.proof__terminal')!
    expect(pre.textContent).toBe('$ echo one\none\ntwo')
    expect(pre.getAttribute('aria-label')).toBe('Output')
    expect(pre.getAttribute('tabindex')).toBe('0')
  })

  it('closes the block with the projected caption, and wraps on demand', async () => {
    const root = await render()
    expect(root.querySelector('figure figcaption')?.textContent?.trim()).toBe('The caption')
    expect(root.querySelector('pre')?.classList.contains('is-wrapping')).toBe(true)
  })

  it('takes one print step per line', async () => {
    const root = await render()
    expect((root.querySelector('pre') as HTMLElement).style.getPropertyValue('--lines')).toBe('3')
  })
})
