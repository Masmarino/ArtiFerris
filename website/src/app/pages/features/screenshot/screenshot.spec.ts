import { Component } from '@angular/core'
import { TestBed } from '@angular/core/testing'
import { setUpTestApp } from '../../../testing/transloco-testing'
import { Screenshot } from './screenshot'

@Component({ imports: [Screenshot], template: '<app-screenshot name="scan" />' })
class Host {}

describe('Screenshot', () => {
  async function render(lang: 'en' | 'fr' | 'de') {
    await setUpTestApp({ imports: [Host], lang })
    const fixture = TestBed.createComponent(Host)
    await fixture.whenStable()
    return fixture.nativeElement as HTMLElement
  }

  it('shows the tight crop in both themes, at its own size so that the layout does not jump', async () => {
    const root = await render('en')
    const image = root.querySelector('img')!

    expect(image.getAttribute('src')).toBe('images/screens/scan-detail-light.webp')
    expect(root.querySelector('source')?.getAttribute('srcset')).toBe(
      'images/screens/scan-detail-dark.webp',
    )
    expect(image.getAttribute('width')).toBe('1860')
    expect(image.getAttribute('height')).toBe('1080')
  })

  it('links to the whole screen, with a visible label and no script needed', async () => {
    const root = await render('fr')
    const link = root.querySelector('a')!

    expect(link.getAttribute('href')).toBe('images/screens/scan-light.webp')
    expect(link.target).toBe('_blank')
    expect(link.rel).toContain('noopener')
    expect(link.textContent).toContain("Voir l'écran complet")
  })

  it('describes the crop, not the page, in the page language', async () => {
    const root = await render('de')
    expect(root.querySelector('img')?.getAttribute('alt')).toContain('Schwachstellen')
  })
})
