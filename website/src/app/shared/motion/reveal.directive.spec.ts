import { Component } from '@angular/core'
import { TestBed } from '@angular/core/testing'
import { FakeIntersectionObserver, stubMatchMedia } from '../../testing/browser-stubs'
import { InView } from './in-view.directive'
import { MotionService } from './motion.service'
import { Reveal } from './reveal.directive'

@Component({
  imports: [Reveal, InView],
  template: `
    <p id="a" [appReveal]="2">first</p>
    <p id="b" appReveal appRevealMode="mark">second</p>
    <p id="c" appInView>loop</p>
  `,
})
class Host {}

describe('Reveal and InView', () => {
  afterEach(() => {
    vi.unstubAllGlobals()
    FakeIntersectionObserver.reset()
    document.documentElement.className = ''
    document.documentElement.removeAttribute('data-tab-hidden')
  })

  async function render(options: { reducedMotion?: boolean; observer?: boolean } = {}) {
    stubMatchMedia({ reducedMotion: options.reducedMotion })
    if (options.observer !== false) vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
    else vi.stubGlobal('IntersectionObserver', undefined)
    TestBed.inject(MotionService).init()
    const fixture = TestBed.createComponent(Host)
    await fixture.whenStable()
    return fixture.nativeElement as HTMLElement
  }

  it('marks the element with data-reveal and a stagger index, and does not show it yet', async () => {
    const root = await render()
    const first = root.querySelector<HTMLElement>('#a')!

    expect(first.hasAttribute('data-reveal')).toBe(true)
    expect(first.style.getPropertyValue('--i')).toBe('2')
    expect(first.classList.contains('is-revealed')).toBe(false)
  })

  it('reveals the element when it enters the viewport, once, and stops observing it', async () => {
    const root = await render()
    const first = root.querySelector<HTMLElement>('#a')!
    const observer = FakeIntersectionObserver.instances[0]

    // Leaving or not yet entering changes nothing.
    observer.trigger(first, false)
    expect(first.classList.contains('is-revealed')).toBe(false)

    observer.trigger(first, true)
    TestBed.tick()
    expect(first.classList.contains('is-revealed')).toBe(true)
    expect(observer.observed.has(first)).toBe(false)
  })

  it('in "mark" mode lights the element without hiding anything', async () => {
    const root = await render()
    const second = root.querySelector<HTMLElement>('#b')!

    expect(second.hasAttribute('data-reveal')).toBe(false)
    FakeIntersectionObserver.instances[0].trigger(second, true)
    TestBed.tick()
    expect(second.classList.contains('is-revealed')).toBe(true)
  })

  it('shows the final state at once, without observing, when motion is reduced', async () => {
    const root = await render({ reducedMotion: true })

    expect(root.querySelector('#a')!.classList.contains('is-revealed')).toBe(true)
    expect(root.querySelector('#b')!.classList.contains('is-revealed')).toBe(true)
    // Only the loop tracker observes; the reveals never did.
    const observed = FakeIntersectionObserver.instances.flatMap((o) => [...o.observed])
    expect(observed.map((element) => element.id)).not.toContain('a')
  })

  it('shows everything when the browser has no IntersectionObserver', async () => {
    const root = await render({ observer: false })
    expect(root.querySelector('#a')!.classList.contains('is-revealed')).toBe(true)
  })

  it('pauses a loop by adding is-offscreen while it is out of the viewport', async () => {
    const root = await render()
    const loop = root.querySelector<HTMLElement>('#c')!
    const observer = FakeIntersectionObserver.instances[0]

    expect(loop.classList.contains('is-offscreen')).toBe(false)
    observer.trigger(loop, false)
    TestBed.tick()
    expect(loop.classList.contains('is-offscreen')).toBe(true)
    observer.trigger(loop, true)
    TestBed.tick()
    expect(loop.classList.contains('is-offscreen')).toBe(false)
  })

  it('sets js-ready on <html> and flags a hidden tab', async () => {
    await render()
    expect(document.documentElement.classList.contains('js-ready')).toBe(true)
    expect(document.documentElement.classList.contains('js')).toBe(true)
    expect(document.documentElement.hasAttribute('data-tab-hidden')).toBe(false)
  })
})

@Component({
  imports: [Reveal, InView],
  template: `<p id="both" appReveal appInView>both</p>`,
})
class BothHost {}

describe('Reveal and InView on the same element', () => {
  afterEach(() => {
    vi.unstubAllGlobals()
    FakeIntersectionObserver.reset()
    document.documentElement.className = ''
  })

  it('serves both: the reveal fires once and the loop tracker keeps listening', async () => {
    stubMatchMedia()
    vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
    const fixture = TestBed.createComponent(BothHost)
    await fixture.whenStable()
    const element = fixture.nativeElement.querySelector('#both') as HTMLElement
    const observer = FakeIntersectionObserver.instances[0]

    observer.trigger(element, true)
    TestBed.tick()
    expect(element.classList.contains('is-revealed')).toBe(true)
    // Still observed for the loop, which now sees it leave.
    expect(observer.observed.has(element)).toBe(true)
    observer.trigger(element, false)
    TestBed.tick()
    expect(element.classList.contains('is-offscreen')).toBe(true)
    expect(element.classList.contains('is-revealed')).toBe(true)
  })
})
