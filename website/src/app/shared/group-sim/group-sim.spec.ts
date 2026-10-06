import { TestBed } from '@angular/core/testing'
import { FakeIntersectionObserver, stubMatchMedia } from '../../testing/browser-stubs'
import { setUpTestApp } from '../../testing/transloco-testing'
import { MotionService } from '../motion/motion.service'
import { GroupSim } from './group-sim'

describe('GroupSim', () => {
  beforeEach(() => setUpTestApp({ imports: [GroupSim], lang: 'en' }))

  afterEach(() => {
    vi.useRealTimers()
    vi.unstubAllGlobals()
    FakeIntersectionObserver.reset()
  })

  /** The look of the two members and of the upstream, in that order. */
  function states(root: HTMLElement): (string | null)[] {
    return Array.from(root.querySelectorAll('.sim__job')).map((node) =>
      node.getAttribute('data-state'),
    )
  }

  function start() {
    vi.useFakeTimers()
    stubMatchMedia({ reducedMotion: false })
    vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
    TestBed.inject(MotionService).init()
    const fixture = TestBed.createComponent(GroupSim)
    fixture.detectChanges()
    const root: HTMLElement = fixture.nativeElement
    FakeIntersectionObserver.instances[0].trigger(root, true)
    return { fixture, root }
  }

  it('shows the .npmrc beside a first request already answered, before any script runs', () => {
    const fixture = TestBed.createComponent(GroupSim)
    fixture.detectChanges()
    const root: HTMLElement = fixture.nativeElement

    expect(root.textContent).toContain('registry=https://acme.artiferris.pro/npm/npm-all/')
    expect(states(root)).toEqual(['skipped', 'success', 'success'])
    expect(root.querySelector('.sim__status')?.textContent).toContain(
      'Package served by npmjs-proxy, from upstream',
    )
    expect(root.querySelector('[role="switch"]')?.getAttribute('aria-checked')).toBe('false')
  })

  it('asks the members in order, then the proxy fetches from upstream', () => {
    const { fixture, root } = start()

    vi.advanceTimersByTime(10)
    fixture.detectChanges()
    expect(states(root)).toEqual(['pending', 'pending', 'pending'])

    vi.advanceTimersByTime(400)
    fixture.detectChanges()
    expect(states(root)).toEqual(['running', 'pending', 'pending'])

    vi.advanceTimersByTime(900)
    fixture.detectChanges()
    expect(states(root)).toEqual(['skipped', 'running', 'pending'])

    vi.advanceTimersByTime(800)
    fixture.detectChanges()
    expect(states(root)).toEqual(['skipped', 'running', 'running'])
    expect(root.textContent).toContain('asks upstream')

    vi.advanceTimersByTime(1400)
    fixture.detectChanges()
    expect(states(root)).toEqual(['skipped', 'success', 'success'])
  })

  it('serves a second request from the proxy, without asking upstream', () => {
    const { fixture, root } = start()
    vi.advanceTimersByTime(4000)

    root.querySelector<HTMLButtonElement>('[role="switch"]')!.click()
    fixture.detectChanges()
    expect(root.querySelector('[role="switch"]')?.getAttribute('aria-checked')).toBe('true')

    vi.advanceTimersByTime(2100)
    fixture.detectChanges()
    expect(states(root)).toEqual(['skipped', 'success', 'pending'])
    expect(root.querySelector('.sim__status')?.textContent).toContain(
      'Package served by npmjs-proxy, from its cache',
    )
    expect(root.textContent).toContain('without asking registry.npmjs.org')
  })
})
