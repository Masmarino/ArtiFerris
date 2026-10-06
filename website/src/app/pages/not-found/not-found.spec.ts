import { TestBed } from '@angular/core/testing'
import { setUpTestApp } from '../../testing/transloco-testing'
import { NotFound } from './not-found'

describe('NotFound', () => {
  it('offers the five home pages as buttons and ends with the same footer as the other pages', async () => {
    await setUpTestApp({ imports: [NotFound], lang: 'en', routes: [] })
    const fixture = TestBed.createComponent(NotFound)
    await fixture.whenStable()
    const root: HTMLElement = fixture.nativeElement

    expect(root.querySelectorAll('h1')).toHaveLength(1)
    const links = Array.from(root.querySelectorAll('main nav a'))
    expect(links.map((link) => link.getAttribute('href'))).toEqual([
      '/en/',
      '/fr/',
      '/it/',
      '/es/',
      '/de/',
    ])
    expect(root.querySelector('app-footer .footer__legal')).not.toBeNull()
  })
})
