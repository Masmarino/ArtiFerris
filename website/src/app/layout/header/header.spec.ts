import { TestBed } from '@angular/core/testing'
import { Router } from '@angular/router'
import { BLANK_ROUTES, setUpTestApp } from '../../testing/transloco-testing'
import { Header } from './header'

describe('Header', () => {
  beforeEach(() => setUpTestApp({ imports: [Header], lang: 'fr', routes: BLANK_ROUTES }))

  async function render() {
    await TestBed.inject(Router).navigateByUrl('/fr')
    const fixture = TestBed.createComponent(Header)
    await fixture.whenStable()
    return fixture
  }

  it('links to the pages of the active language and to the external destinations', async () => {
    const fixture = await render()
    const root: HTMLElement = fixture.nativeElement
    const nav = root.querySelector('nav')!

    const hrefs = Array.from(nav.querySelectorAll('a')).map((link) => link.getAttribute('href'))
    expect(hrefs).toEqual([
      '/fr/features/',
      '/fr/registries/',
      '/fr/install/',
      '/fr/security/',
      '/fr/roadmap/',
      'https://app.artiferris.pro/docs',
    ])
    expect(nav.textContent).toContain('Produit')
    expect(nav.textContent).toContain('Feuille de route')
    expect(nav.textContent).toContain('Sécurité')
    expect(root.querySelector('.header__brand')?.getAttribute('href')).toBe('/fr/')
  })

  it('opens GitHub in a new tab, safely, and the app from the main button', async () => {
    const fixture = await render()
    const root: HTMLElement = fixture.nativeElement
    const github = root.querySelector<HTMLAnchorElement>('.header__github')!
    expect(github.getAttribute('href')).toBe('https://github.com/Masmarino/ArtiFerris')
    expect(github.target).toBe('_blank')
    expect(github.rel).toContain('noopener')
    expect(github.getAttribute('aria-label')).toBe('GitHub')

    const cta = root.querySelector<HTMLAnchorElement>('.header__cta')!
    expect(cta.getAttribute('href')).toBe('https://app.artiferris.pro')
    expect(cta.textContent?.trim()).toBe('Ouvrir ArtiFerris')
  })

  it('toggles the menu on small screens and closes it with Escape', async () => {
    const fixture = await render()
    const root: HTMLElement = fixture.nativeElement
    const toggle = root.querySelector<HTMLButtonElement>('.header__toggle')!
    const panel = root.querySelector('#site-menu')!

    expect(toggle.getAttribute('aria-expanded')).toBe('false')
    toggle.click()
    await fixture.whenStable()
    expect(toggle.getAttribute('aria-expanded')).toBe('true')
    expect(panel.classList.contains('is-open')).toBe(true)

    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    await fixture.whenStable()
    expect(toggle.getAttribute('aria-expanded')).toBe('false')
  })

  it('contains the language switcher and the theme toggle', async () => {
    const fixture = await render()
    const root: HTMLElement = fixture.nativeElement
    expect(root.querySelector('app-language-switcher')).not.toBeNull()
    expect(root.querySelector('app-theme-toggle')).not.toBeNull()
  })

  it('gets a backdrop and shadow once the page has scrolled, and loses them back at the top', async () => {
    // Frames run when the test says so, like the browser's: one at a time, after the event handler returned.
    const frames: FrameRequestCallback[] = []
    vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) =>
      frames.push(callback),
    )
    const flushFrame = () => frames.splice(0).forEach((callback) => callback(0))
    const fixture = await render()
    const header: HTMLElement = fixture.nativeElement.querySelector('.header')
    expect(header.classList.contains('is-scrolled')).toBe(false)

    Object.defineProperty(window, 'scrollY', { configurable: true, value: 120 })
    window.dispatchEvent(new Event('scroll'))
    flushFrame()
    await fixture.whenStable()
    expect(header.classList.contains('is-scrolled')).toBe(true)

    Object.defineProperty(window, 'scrollY', { configurable: true, value: 0 })
    window.dispatchEvent(new Event('scroll'))
    flushFrame()
    await fixture.whenStable()
    expect(header.classList.contains('is-scrolled')).toBe(false)
    vi.unstubAllGlobals()
  })
})
