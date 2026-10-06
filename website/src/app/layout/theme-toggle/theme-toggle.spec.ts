import { TestBed } from '@angular/core/testing'
import { stubMatchMedia } from '../../testing/browser-stubs'
import { setUpTestApp } from '../../testing/transloco-testing'
import { THEME_STORAGE_KEY, ThemeStore } from '../../theme/theme-store'
import { ThemeToggle } from './theme-toggle'

describe('ThemeToggle', () => {
  beforeEach(async () => {
    window.localStorage.clear()
    document.documentElement.removeAttribute('data-theme')
    await setUpTestApp({ imports: [ThemeToggle], lang: 'en' })
  })

  it('offers the dark theme on a light system and stores the choice when clicked', async () => {
    stubMatchMedia()
    TestBed.inject(ThemeStore).init()
    const fixture = TestBed.createComponent(ThemeToggle)
    await fixture.whenStable()

    const button: HTMLButtonElement = fixture.nativeElement.querySelector('button')
    expect(button.getAttribute('aria-label')).toBe('Switch to the dark theme')

    button.click()
    await fixture.whenStable()

    expect(document.documentElement.getAttribute('data-theme')).toBe('dark')
    expect(window.localStorage.getItem(THEME_STORAGE_KEY)).toBe('dark')
    expect(button.getAttribute('aria-label')).toBe('Switch to the light theme')
  })

  it('follows a dark system until the visitor chooses', async () => {
    stubMatchMedia({ dark: true })
    const store = TestBed.inject(ThemeStore)
    store.init()
    const fixture = TestBed.createComponent(ThemeToggle)
    await fixture.whenStable()

    expect(store.effective()).toBe('dark')
    expect(document.documentElement.hasAttribute('data-theme')).toBe(false)
    fixture.nativeElement.querySelector('button').click()
    await fixture.whenStable()

    expect(document.documentElement.getAttribute('data-theme')).toBe('light')
    expect(window.localStorage.getItem(THEME_STORAGE_KEY)).toBe('light')
  })

  it('shows both glyphs in the markup, so the prerendered page needs no script to pick one', async () => {
    stubMatchMedia()
    const fixture = TestBed.createComponent(ThemeToggle)
    await fixture.whenStable()
    expect(fixture.nativeElement.querySelectorAll('gbt-icon')).toHaveLength(2)
  })
})
