import { TestBed } from '@angular/core/testing'
import { Router } from '@angular/router'
import { stubMatchMedia } from '../../testing/browser-stubs'
import { BLANK_ROUTES, setUpTestApp } from '../../testing/transloco-testing'
import { Registries } from './registries'

describe('Registries', () => {
  beforeEach(async () => {
    stubMatchMedia()
    await setUpTestApp({ imports: [Registries], lang: 'en', routes: BLANK_ROUTES })
    await TestBed.inject(Router).navigateByUrl('/en/registries')
  })

  async function render() {
    const fixture = TestBed.createComponent(Registries)
    await fixture.whenStable()
    return fixture.nativeElement as HTMLElement
  }

  it('has one h1, and a path that says where the page sits', async () => {
    const root = await render()

    expect(root.querySelectorAll('h1')).toHaveLength(1)
    expect(root.querySelector('h1')?.textContent).toBe('Two registries, one console')
    expect(root.querySelector('.crumb')?.textContent?.replace(/\s+/g, ' ').trim()).toBe(
      'ArtiFerris / Registries',
    )
    expect(root.querySelector('.crumb a')?.getAttribute('href')).toBe('/en/')
  })

  it('plays an install through a group, shows the Docker commands, then the rules of a repository', async () => {
    const root = await render()

    expect(root.querySelector('app-group-sim')).not.toBeNull()
    expect(root.querySelector('#docker')?.textContent).toContain(
      'docker push acme.artiferris.pro/images/api:1.4.0',
    )
    const terms = Array.from(root.querySelectorAll('.spec__row dt')).map((dt) =>
      dt.textContent?.trim(),
    )
    expect(terms).toEqual([
      'Three types of repository',
      'A quota per repository',
      'Retention per repository',
      'Dependencies audited',
      'Images analyzed',
      'Public repositories',
    ])
    for (const page of ['utilisation/npm', 'utilisation/docker', 'utilisation/depots']) {
      expect(root.querySelector(`a[href="https://app.artiferris.pro/docs/${page}"]`)).not.toBeNull()
    }
  })

  it('ends on the call to try or install', async () => {
    const root = await render()

    expect(root.querySelector('app-cta .gbt-button--primary')?.getAttribute('href')).toBe(
      'https://app.artiferris.pro',
    )
  })
})
