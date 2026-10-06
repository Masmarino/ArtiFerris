import { Component } from '@angular/core'
import { TestBed } from '@angular/core/testing'
import { Window } from './window'

@Component({
  imports: [Window],
  template: `<app-window url="git.example.com/acme/api"><p>shot</p></app-window>`,
})
class Host {}

describe('Window', () => {
  it('wraps its content and hides its title bar from assistive technology', () => {
    const fixture = TestBed.createComponent(Host)
    fixture.detectChanges()
    const frame: HTMLElement = fixture.nativeElement.querySelector('app-window')

    expect(frame.classList.contains('window')).toBe(true)
    expect(frame.querySelector('.window__body p')?.textContent).toBe('shot')
    expect(frame.querySelector('.window__bar')?.getAttribute('aria-hidden')).toBe('true')
    expect(frame.querySelector('.window__url')?.textContent).toBe('git.example.com/acme/api')
  })
})
