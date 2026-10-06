import { TestBed } from '@angular/core/testing'
import { GitField } from './git-field'

describe('GitField', () => {
  function render(variant: 'hero' | 'band' = 'hero') {
    TestBed.configureTestingModule({ imports: [GitField] })
    const fixture = TestBed.createComponent(GitField)
    fixture.componentRef.setInput('variant', variant)
    fixture.detectChanges()
    return fixture.nativeElement as HTMLElement
  }

  it('is decoration, hidden from assistive technology', () => {
    const root = render()

    expect(root.getAttribute('aria-hidden')).toBe('true')
    expect(root.querySelector('svg')?.getAttribute('focusable')).toBe('false')
  })

  it('draws the same commit graph twice, side by side, so that it loops without a seam', () => {
    const root = render()
    const tiles = root.querySelectorAll('.git-field__track > g')

    expect(tiles).toHaveLength(2)
    expect(tiles[0].innerHTML).toBe(tiles[1].innerHTML)
    expect(tiles[0].querySelectorAll('.git-field__line').length).toBeGreaterThan(7)
    expect(tiles[0].querySelectorAll('.git-field__commit').length).toBeGreaterThan(30)
    expect(tiles[0].querySelectorAll('.git-field__pulse').length).toBeGreaterThan(2)
  })

  it('draws the same graph every time, so the server and the browser agree', () => {
    const first = render().querySelector('.git-field__track')!.innerHTML
    TestBed.resetTestingModule()
    const second = render().querySelector('.git-field__track')!.innerHTML

    expect(second).toBe(first)
  })

  it('is fainter behind the title of an inner page', () => {
    expect(render('band').classList.contains('git-field--quiet')).toBe(true)
  })
})
