import { ChangeDetectionStrategy, Component, input } from '@angular/core'

/** Width and height of one tile of the drawing; two tiles side by side scroll by one width and loop without a seam. */
const W = 1600
const H = 560
const LANES = 7
const laneY = (lane: number) => 50 + lane * ((H - 100) / (LANES - 1))

// Each lane has a colour of the site: the crab's rust, its amber, the product's blue and the patina green.
const COLORS = ['brand', 'indigo', 'patina', 'brand-2', 'indigo', 'patina', 'brand'] as const
type Color = (typeof COLORS)[number]

interface Line {
  d: string
  color: Color
}

interface Commit {
  x: number
  y: number
  color: Color
  hollow: boolean
  /** Seconds of delay of its blink, so that the commits do not all light up together. */
  delay: number
}

interface Pulse {
  d: string
  color: Color
  duration: number
  delay: number
}

/** A small seeded generator: the drawing is the same on the server and in the browser, so hydration keeps it. */
function random(seed: number): () => number {
  let state = seed
  return () => {
    state = (state + 0x6d2b79f5) | 0
    let t = Math.imul(state ^ (state >>> 15), 1 | state)
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

/**
 * A commit graph drawn across the tile: one straight line per lane, branches that leave a lane for the next one and
 * merge back, commits along them. Every line is straight at both edges, so the second tile carries on the first.
 */
function draw(): { lines: Line[]; commits: Commit[]; pulses: Pulse[] } {
  const next = random(20261004)
  const lines: Line[] = []
  const commits: Commit[] = []
  const pulses: Pulse[] = []

  for (let lane = 0; lane < LANES; lane++) {
    const y = laneY(lane)
    const color = COLORS[lane]
    lines.push({ d: `M0 ${y}H${W}`, color })
    for (let x = 40 + next() * 120; x < W - 30; x += 110 + next() * 170) {
      commits.push({ x, y, color, hollow: next() < 0.3, delay: next() * 6 })
    }
    if (lane % 2 === 0) {
      pulses.push({ d: `M0 ${y}H${W}`, color, duration: 7 + next() * 6, delay: -next() * 10 })
    }
  }

  // Branches: from a lane to its neighbour and back, never crossing the tile's edges.
  for (let n = 0; n < 9; n++) {
    const lane = Math.floor(next() * LANES)
    const to = lane === 0 ? 1 : lane === LANES - 1 ? LANES - 2 : lane + (next() < 0.5 ? -1 : 1)
    const x0 = 80 + next() * (W - 520)
    const length = 140 + next() * 220
    const [y0, y1] = [laneY(lane), laneY(to)]
    const color = COLORS[to]
    lines.push({
      d: `M${x0} ${y0}C${x0 + 50} ${y0} ${x0 + 40} ${y1} ${x0 + 90} ${y1}H${x0 + 90 + length}C${x0 + 140 + length} ${y1} ${x0 + 130 + length} ${y0} ${x0 + 180 + length} ${y0}`,
      color,
    })
    commits.push({ x: x0 + 90 + length / 2, y: y1, color, hollow: false, delay: next() * 6 })
  }
  return { lines, commits, pulses }
}

const DRAWING = draw()

/**
 * The coloured background of the dark bands: a commit graph that drifts slowly sideways, its commits blinking in turn
 * and pushes running along some lanes, over two soft fields of colour. Decoration only: hidden from assistive
 * technology, under the content, and still when reduced motion is asked for (the global rule stops every animation).
 */
@Component({
  selector: 'app-git-field',
  templateUrl: './git-field.html',
  styleUrl: './git-field.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: {
    class: 'git-field loop',
    'aria-hidden': 'true',
    '[class.git-field--quiet]': "variant() === 'band'",
  },
})
export class GitField {
  /** `hero` is the full drawing; `band`, for the title block of the inner pages, is fainter. */
  readonly variant = input<'hero' | 'band'>('hero')

  protected readonly width = W
  protected readonly height = H
  protected readonly tiles = [0, W]
  protected readonly drawing = DRAWING
}
