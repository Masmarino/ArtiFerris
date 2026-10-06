import { TestBed } from '@angular/core/testing'
import { Router } from '@angular/router'
import { BLANK_ROUTES, setUpTestApp } from '../../testing/transloco-testing'
import { Features } from './features'

describe('Features tabs', () => {
  beforeEach(async () => {
    await setUpTestApp({ imports: [Features], lang: 'en', routes: BLANK_ROUTES })
    await TestBed.inject(Router).navigateByUrl('/en/features')
  })

  it('renders every panel, with only the first one active', () => {
    const fixture = TestBed.createComponent(Features)
    fixture.detectChanges()
    const root: HTMLElement = fixture.nativeElement

    const panels = root.querySelectorAll('.tabs__panel')
    expect(panels).toHaveLength(6)
    expect(Array.from(panels).map((panel) => panel.classList.contains('is-active'))).toEqual([
      true,
      false,
      false,
      false,
      false,
      false,
    ])
    // Every panel keeps its heading and text in the markup, so they all read without JavaScript.
    expect(root.querySelectorAll('.tabs__panel h2')).toHaveLength(6)
    expect(root.querySelectorAll('.tabs__panel .check-list li').length).toBeGreaterThan(20)
  })

  it('becomes an ARIA tab set once scripts run', async () => {
    const fixture = TestBed.createComponent(Features)
    await fixture.whenStable()
    const root: HTMLElement = fixture.nativeElement

    expect(root.querySelector('[role="tablist"]')).not.toBeNull()
    const tabs = Array.from(root.querySelectorAll<HTMLElement>('[role="tab"]'))
    expect(tabs).toHaveLength(6)
    expect(tabs.map((tab) => tab.getAttribute('aria-selected'))).toEqual([
      'true',
      'false',
      'false',
      'false',
      'false',
      'false',
    ])
    // Roving tabindex: only the selected tab is in the tab order.
    expect(tabs.map((tab) => tab.getAttribute('tabindex'))).toEqual([
      null,
      '-1',
      '-1',
      '-1',
      '-1',
      '-1',
    ])
    expect(tabs[0].getAttribute('aria-controls')).toBe('panel-repos')
    expect(root.querySelector('#panel-repos')?.getAttribute('role')).toBe('tabpanel')
    expect(root.querySelector('#panel-repos')?.getAttribute('aria-labelledby')).toBe('tab-repos')
    expect(tabs[3].textContent?.trim()).toBe('Organizations')
    expect(root.querySelectorAll('app-window app-screenshot')).toHaveLength(6)
  })

  it('selects a tab with a click and shows its panel', async () => {
    const fixture = TestBed.createComponent(Features)
    await fixture.whenStable()
    const root: HTMLElement = fixture.nativeElement

    root.querySelector<HTMLElement>('#tab-packages')!.click()
    await fixture.whenStable()

    expect(root.querySelector('#tab-packages')?.getAttribute('aria-selected')).toBe('true')
    expect(root.querySelector('#panel-packages')?.classList.contains('is-active')).toBe(true)
    expect(root.querySelector('#panel-repos')?.classList.contains('is-active')).toBe(false)
  })

  it('moves between tabs with the arrows, Home and End, and focuses the new tab', async () => {
    const fixture = TestBed.createComponent(Features)
    document.body.appendChild(fixture.nativeElement)
    await fixture.whenStable()
    const root: HTMLElement = fixture.nativeElement
    const press = async (key: string) => {
      const current = root.querySelector<HTMLElement>('[role="tab"][aria-selected="true"]')!
      current.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true }))
      await fixture.whenStable()
      return root.querySelector('[role="tab"][aria-selected="true"]')!
    }

    expect((await press('ArrowRight')).id).toBe('tab-packages')
    expect(document.activeElement?.id).toBe('tab-packages')
    expect((await press('End')).id).toBe('tab-docs')
    expect((await press('ArrowRight')).id).toBe('tab-repos')
    expect((await press('ArrowLeft')).id).toBe('tab-docs')
    expect((await press('Home')).id).toBe('tab-repos')

    fixture.nativeElement.remove()
  })

  it('opens the tab named by the URL fragment', async () => {
    window.location.hash = '#feature-scan'
    const fixture = TestBed.createComponent(Features)
    await fixture.whenStable()
    expect(
      fixture.nativeElement.querySelector('#panel-scan')?.classList.contains('is-active'),
    ).toBe(true)
    window.location.hash = ''
  })

  it('keeps the heading ids that existing links point at', () => {
    const fixture = TestBed.createComponent(Features)
    fixture.detectChanges()
    const ids = Array.from(
      (fixture.nativeElement as HTMLElement).querySelectorAll<HTMLElement>('[id^="feature-"]'),
    ).map((element) => element.id)
    for (const id of [
      'repos',
      'packages',
      'scan',
      'tenants',
      'public',
      'docs',
      'accounts',
      'admin',
      'languages',
      'tech',
    ]) {
      expect(ids).toContain(`feature-${id}`)
    }
  })
})
