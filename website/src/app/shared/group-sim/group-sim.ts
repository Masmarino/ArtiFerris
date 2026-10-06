import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  ElementRef,
  afterNextRender,
  computed,
  inject,
  signal,
} from '@angular/core'
import { TranslocoPipe } from '@jsverse/transloco'
import { JobStatus } from '@masmarino/gabarit'
import { CodeBlock } from '../code-block/code-block'
import { MotionService } from '../motion/motion.service'
import { ViewportObserver } from '../motion/viewport-observer'
import { NPMRC_EXAMPLE } from '../snippets'

type Member = 'idle' | 'checking' | 'miss' | 'upstream' | 'served' | 'cached'
type Upstream = 'idle' | 'fetching' | 'done'
type Status = 'received' | 'servedUpstream' | 'servedCache'

interface Snapshot {
  hosted: Member
  proxy: Member
  upstream: Upstream
  status: Status
}

const COLD_DONE: Snapshot = {
  hosted: 'miss',
  proxy: 'served',
  upstream: 'done',
  status: 'servedUpstream',
}
const WARM_DONE: Snapshot = {
  hosted: 'miss',
  proxy: 'cached',
  upstream: 'idle',
  status: 'servedCache',
}

// What a group does with `npm install left-pad`, as [ms from the start of a request, state]. Its members are asked in
// the order set on the group, and the first one that holds the package answers; a proxy fetches what it does not hold
// yet from its upstream and keeps it (docs/utilisation/depots.md).
function timeline(warm: boolean): [number, Snapshot][] {
  const steps: [number, Snapshot][] = [
    [0, { hosted: 'idle', proxy: 'idle', upstream: 'idle', status: 'received' }],
    [350, { hosted: 'checking', proxy: 'idle', upstream: 'idle', status: 'received' }],
    [1200, { hosted: 'miss', proxy: 'checking', upstream: 'idle', status: 'received' }],
  ]
  if (warm) {
    steps.push([2000, WARM_DONE])
  } else {
    steps.push([
      2000,
      { hosted: 'miss', proxy: 'upstream', upstream: 'fetching', status: 'received' },
    ])
    steps.push([3400, COLD_DONE])
  }
  return steps
}

/** The look of each state, borrowed from the job states of the stylesheet and of `gbt-job-status`. */
const LOOK: Record<
  Member | Upstream,
  { state: string; glyph: 'pending' | 'running' | 'success' | 'canceled' }
> = {
  idle: { state: 'pending', glyph: 'pending' },
  checking: { state: 'running', glyph: 'running' },
  miss: { state: 'skipped', glyph: 'canceled' },
  upstream: { state: 'running', glyph: 'running' },
  fetching: { state: 'running', glyph: 'running' },
  served: { state: 'success', glyph: 'success' },
  cached: { state: 'success', glyph: 'success' },
  done: { state: 'success', glyph: 'success' },
}

/**
 * The registries demo: the `.npmrc` of a project pointed at a group, next to what the group does with an install. It
 * plays when it scrolls into view; the switch replays it as a second request, which the proxy serves from what it kept.
 * The prerendered page and reduced motion show the finished first request.
 */
@Component({
  selector: 'app-group-sim',
  imports: [TranslocoPipe, JobStatus, CodeBlock],
  templateUrl: './group-sim.html',
  styleUrl: './group-sim.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class GroupSim {
  private readonly motion = inject(MotionService)
  private readonly viewport = inject(ViewportObserver)
  private readonly element: HTMLElement = inject(ElementRef).nativeElement
  private timers: number[] = []

  protected readonly code = NPMRC_EXAMPLE
  protected readonly warm = signal(false)
  protected readonly state = signal<Snapshot>(COLD_DONE)
  protected readonly statusGlyph = computed(() =>
    this.state().status === 'received' ? 'running' : 'success',
  )
  protected readonly members = computed(() => {
    const s = this.state()
    return [
      { name: 'npm-hosted', type: 'hosted', phase: s.hosted, ...LOOK[s.hosted] },
      { name: 'npmjs-proxy', type: 'proxy', phase: s.proxy, ...LOOK[s.proxy] },
    ]
  })
  protected readonly upstream = computed(() => ({
    phase: this.state().upstream,
    ...LOOK[this.state().upstream],
  }))
  /** The request line from the proxy to its upstream: travels while it fetches, drawn once it has answered. */
  protected readonly link = computed(() => LOOK[this.state().upstream].state)

  constructor() {
    afterNextRender(() => {
      if (this.motion.reducedMotion() || !this.viewport.supported) return
      this.viewport.observeOnce(this.element, () => this.play())
    })
    inject(DestroyRef).onDestroy(() => {
      this.clear()
      this.viewport.unobserve(this.element)
    })
  }

  protected toggle(): void {
    this.warm.update((value) => !value)
    this.play()
  }

  protected play(): void {
    this.clear()
    const steps = timeline(this.warm())
    if (this.motion.reducedMotion()) {
      this.state.set(steps[steps.length - 1][1])
      return
    }
    for (const [ms, snapshot] of steps) {
      this.timers.push(window.setTimeout(() => this.state.set(snapshot), ms))
    }
  }

  private clear(): void {
    for (const timer of this.timers) window.clearTimeout(timer)
    this.timers = []
  }
}
