import { TestBed } from '@angular/core/testing'
import { Router } from '@angular/router'
import { BLANK_ROUTES, setUpTestApp } from '../../testing/transloco-testing'
import { Footer } from './footer'

describe('Footer', () => {
  async function render(lang: 'en' | 'fr') {
    await setUpTestApp({ imports: [Footer], lang, routes: BLANK_ROUTES })
    await TestBed.inject(Router).navigateByUrl(`/${lang}`)
    const fixture = TestBed.createComponent(Footer)
    await fixture.whenStable()
    return fixture.nativeElement as HTMLElement
  }

  it('has three columns, the exact license mention and no version or date', async () => {
    const root = await render('en')

    expect(root.querySelectorAll('.footer__col')).toHaveLength(3)
    expect(root.textContent).toContain('Licensed under the Apache License 2.0.')
    expect(root.textContent).toContain('No cookies, no tracking')
    expect(root.querySelector('.footer__legal')?.textContent).not.toMatch(/\d+\.\d+\.\d+|20\d\d/)
  })

  it('links to the pages in the active language and to the outside resources', async () => {
    const root = await render('fr')
    const hrefs = Array.from(root.querySelectorAll('a')).map((link) => link.getAttribute('href'))

    expect(hrefs).toEqual(
      expect.arrayContaining([
        '/fr/features/',
        '/fr/registries/',
        '/fr/install/',
        '/fr/security/',
        '/fr/roadmap/',
        'https://app.artiferris.pro',
        'https://app.artiferris.pro/docs/administration/installation',
        'https://app.artiferris.pro/docs/utilisation/npm',
        'https://app.artiferris.pro/docs/utilisation/docker',
        'https://github.com/Masmarino/ArtiFerris',
        'https://github.com/Masmarino/ArtiFerris/blob/main/CHANGELOG.md',
        'https://www.ferrisgit.pro',
      ]),
    )
    expect(root.textContent).toContain('Aucun cookie, aucun pistage')
  })
})
