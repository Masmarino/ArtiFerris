import { TestBed } from '@angular/core/testing'
import { Lang } from '../../i18n/languages'
import { setUpTestApp } from '../../testing/transloco-testing'
import { PlanHighlight } from '../plan-highlight'
import { ArchitecturePlan } from './architecture-plan'

describe('ArchitecturePlan', () => {
  async function renderPlan(lang: Lang = 'en') {
    await setUpTestApp({ imports: [ArchitecturePlan], lang, routes: [] })
    const fixture = TestBed.createComponent(ArchitecturePlan)
    await fixture.whenStable()
    return fixture
  }

  async function render(lang: Lang = 'en') {
    return (await renderPlan(lang)).nativeElement as HTMLElement
  }

  it('is one image with a title and a description in the page language', async () => {
    const root = await render('fr')
    const svg = root.querySelector('svg')!

    expect(svg.getAttribute('role')).toBe('img')
    expect(root.querySelector('#plan-title')?.textContent).toBe(
      "Architecture d'une instance ArtiFerris",
    )
    expect(root.querySelector('#plan-desc')?.textContent).toContain('Dessin isométrique')
    expect(svg.getAttribute('aria-labelledby')).toBe('plan-title plan-desc')
  })

  it('draws its labels as real, translated text', async () => {
    const english = await render('en')
    const text = english.textContent ?? ''
    for (const label of [
      'npm or Docker client',
      'Browser',
      'Upstream registries',
      'Directories and mail',
      'npm install',
    ]) {
      expect(text).toContain(label)
    }
    // Names of crates and stores are identifiers: they are the same in every language.
    for (const name of [
      'api',
      'npm',
      'docker',
      'application',
      'domain',
      'infrastructure',
      'PostgreSQL',
    ]) {
      expect(text).toContain(name)
    }
  })

  it('shows no raw translation key and keeps its hatching', async () => {
    const root = await render('de')
    expect(root.textContent).not.toMatch(/plan\.[a-z]+\./)
    expect(root.textContent).toContain('Upstream-Registries')
    expect(root.querySelectorAll('pattern')).toHaveLength(2)
  })

  it('has seven numbered circles that lead to the tiles of the home page, and light them', async () => {
    const fixture = await renderPlan()
    const root: HTMLElement = fixture.nativeElement
    const circles = Array.from(root.querySelectorAll<SVGAElement>('a.bub'))

    expect(circles.map((c) => c.textContent?.trim())).toEqual(['1', '2', '3', '4', '5', '6', '7'])
    expect(circles.map((c) => c.getAttribute('href'))).toEqual(
      [1, 2, 3, 4, 5, 6, 7].map((n) => `/en/#d${n}`),
    )
    // They are an addition for the mouse: the keyboard uses the list under the drawing.
    expect(circles.every((c) => c.getAttribute('tabindex') === '-1')).toBe(true)

    TestBed.inject(PlanHighlight).set(3)
    await fixture.whenStable()
    expect(root.querySelectorAll('a.bub.on')).toHaveLength(1)
    expect(root.querySelector('a.bub.on')?.textContent?.trim()).toBe('3')
  })

  it('carries the flow of an install, to the upstream registry', async () => {
    const root = await render('en')
    expect(root.querySelectorAll('.flow polyline').length).toBeGreaterThanOrEqual(2)
    expect(root.querySelectorAll('.pulses polyline').length).toBeGreaterThanOrEqual(2)
  })
})
