// Generates the template of the architecture plan, the isometric drawing of an ArtiFerris instance on the home page, as
// one inline SVG. The geometry is computed here from world coordinates; the labels are Transloco
// bindings, so they are real text in the prerendered pages, in every language.
//
//     npm run plan
//
// The output, src/app/shared/architecture-plan/architecture-plan.html, is committed and a normal build does not run this
// script: edit the scene below, not the generated file. The animation delays (--d) are consumed by
// architecture-plan.scss. scripts/build-images.mjs reuses the output for the social preview.
import { writeFileSync } from 'node:fs'
import { join } from 'node:path'
import prettier from 'prettier'
import { root } from './paths.mjs'

const output = join(root, 'src/app/shared/architecture-plan/architecture-plan.html')

// Isometric projection from world (x, y, z) to screen; one world unit is K px along the axes.
const K = 16
const C = Math.sqrt(3) / 2
const P = (x, y, z = 0) => [(x - y) * C * K, (x + y) * 0.5 * K - z * K]
const project = (points) => points.map((p) => P(...p))
const f = (n) => Math.round(n * 100) / 100
const pt = (p) => `${f(p[0])},${f(p[1])}`
const poly = (pts) => pts.map(pt).join(' ')
const delay = (d) => `style="--d:${f(d)}s"`

/** A label that comes from the translations. */
const t = (key) => `{{ '${key}' | transloco }}`

const out = []
const push = (s) => out.push(s)

// pathLength="1" lets the stylesheet animate every outline with the same dash values, whatever its real length.
const polygon = (pts, cls, d) =>
  `<polygon class="dr ${cls}" pathLength="1" points="${poly(pts)}" ${delay(d)}/>`

/** An isometric box. Visible faces: left (+y), right (+x) and top. */
function box(x0, y0, z0, x1, y1, z1, { d = 0, cls = '' } = {}) {
  const top = [P(x0, y0, z1), P(x1, y0, z1), P(x1, y1, z1), P(x0, y1, z1)]
  const left = [P(x0, y1, z0), P(x1, y1, z0), P(x1, y1, z1), P(x0, y1, z1)]
  const right = [P(x1, y0, z0), P(x1, y1, z0), P(x1, y1, z1), P(x1, y0, z1)]
  return (
    `<g class="part ${cls}">` +
    polygon(left, 'f ln', d) +
    polygon(right, 'f ln', d + 0.08) +
    polygon(top, 'f ln top', d + 0.16) +
    `<polygon class="hat hl" points="${poly(left)}" ${delay(d + 1)}/>` +
    `<polygon class="hat hr" points="${poly(right)}" ${delay(d + 1)}/>` +
    '</g>'
  )
}

/** Vertical cylinder, drawn as a side, a top ellipse, a band and the hatching of its shaded side. */
function cylinder(cx, cy, r, z0, z1, d) {
  const rx = r * Math.SQRT2 * C * K
  const ry = r * Math.SQRT2 * 0.5 * K
  const [sx, sy0] = P(cx, cy, z0)
  const [, sy1] = P(cx, cy, z1)
  const mid = (sy0 + sy1) / 2
  return (
    '<g class="part">' +
    `<path class="dr f ln" pathLength="1" d="M${f(sx - rx)},${f(sy1)} V${f(sy0)} A${f(rx)},${f(ry)} 0 0 0 ${f(sx + rx)},${f(sy0)} V${f(sy1)}" ${delay(d)}/>` +
    `<ellipse class="dr f ln top" pathLength="1" cx="${f(sx)}" cy="${f(sy1)}" rx="${f(rx)}" ry="${f(ry)}" ${delay(d + 0.15)}/>` +
    `<path class="fade ln thin" d="M${f(sx - rx)},${f(mid)} A${f(rx)},${f(ry)} 0 0 0 ${f(sx + rx)},${f(mid)}" ${delay(d + 0.9)}/>` +
    `<path class="hat hr" d="M${f(sx)},${f(sy1 + ry)} A${f(rx)},${f(ry)} 0 0 0 ${f(sx + rx)},${f(sy1)} V${f(sy0)} A${f(rx)},${f(ry)} 0 0 1 ${f(sx)},${f(sy0 + ry)} Z" ${delay(d + 1)}/>` +
    '</g>'
  )
}

/** Text lying on the +y face (the lower left one), along x. */
function faceText(x, y, z, str, d, cls = 'tx') {
  const [sx, sy] = P(x, y, z)
  return `<text class="${cls}" transform="matrix(${f(C)} 0.5 0 1 ${f(sx)} ${f(sy)})" ${delay(d)}>${str}</text>`
}

const text = (sx, sy, str, d, cls = 'tx', anchor = 'start') =>
  `<text class="${cls}" x="${f(sx)}" y="${f(sy)}" text-anchor="${anchor}" ${delay(d)}>${str}</text>`

/** Leader line from a world point to a screen point, ending with a dot on the part. */
const leader = (p, to, d) =>
  `<polyline class="dr ld" pathLength="1" points="${pt(p)} ${pt(to)}" ${delay(d)}/>` +
  `<circle class="dot" cx="${f(p[0])}" cy="${f(p[1])}" r="2" ${delay(d + 0.6)}/>`

function arrowHead(a, b, size = 7) {
  const dx = b[0] - a[0]
  const dy = b[1] - a[1]
  const length = Math.hypot(dx, dy)
  const ux = dx / length
  const uy = dy / length
  const bx = b[0] - ux * size
  const by = b[1] - uy * size
  return `${f(bx - uy * size * 0.45)},${f(by + ux * size * 0.45)} ${f(b[0])},${f(b[1])} ${f(bx + uy * size * 0.45)},${f(by - ux * size * 0.45)}`
}

/** A link between world points, with an arrow head. Dashed links fade in; solid ones are drawn. */
function route(pts, { d = 0, dashed = false } = {}) {
  const sp = project(pts)
  const line = dashed
    ? `<polyline class="fade ln dash" points="${poly(sp)}" ${delay(d)}/>`
    : `<polyline class="dr ln" pathLength="1" points="${poly(sp)}" ${delay(d)}/>`
  return `<g class="part">${line}<polygon class="ahead ln" points="${arrowHead(sp[sp.length - 2], sp[sp.length - 1])}" ${delay(d + 0.9)}/></g>`
}

/** Dimension line between two world points, offset sideways, with the label on the line. */
function dimension(a, b, off, label, d, labelWidth) {
  const A = P(...a)
  const B = P(...b)
  const A2 = P(a[0] + off[0], a[1] + off[1], a[2])
  const B2 = P(b[0] + off[0], b[1] + off[1], b[2])
  const mx = (A2[0] + B2[0]) / 2
  const my = (A2[1] + B2[1]) / 2
  const angle = (Math.atan2(B2[1] - A2[1], B2[0] - A2[0]) * 180) / Math.PI
  const tick = (p, q) => {
    const length = Math.hypot(q[0] - p[0], q[1] - p[1])
    const ux = (q[0] - p[0]) / length
    const uy = (q[1] - p[1]) / length
    return `${f(p[0] - ux * 5 - uy * 4)},${f(p[1] - uy * 5 + ux * 4)} ${f(p[0] + ux * 5 + uy * 4)},${f(p[1] + uy * 5 - ux * 4)}`
  }
  const length = Math.hypot(B2[0] - A2[0], B2[1] - A2[1])
  // The gap fits the longest label of the five languages, so the line never runs through the text.
  const gap = Math.min(labelWidth, length - 10)
  const ux = (B2[0] - A2[0]) / length
  const uy = (B2[1] - A2[1]) / length
  const g1 = [mx - (ux * gap) / 2, my - (uy * gap) / 2]
  const g2 = [mx + (ux * gap) / 2, my + (uy * gap) / 2]
  const extend = (from, to) =>
    `x2="${f(to[0] + (to[0] - from[0]) * 0.12)}" y2="${f(to[1] + (to[1] - from[1]) * 0.12)}"`
  return (
    '<g class="part dim">' +
    `<line class="fade thin" x1="${f(A[0])}" y1="${f(A[1])}" ${extend(A, A2)} ${delay(d)}/>` +
    `<line class="fade thin" x1="${f(B[0])}" y1="${f(B[1])}" ${extend(B, B2)} ${delay(d)}/>` +
    `<line class="dr thin" pathLength="1" x1="${f(A2[0])}" y1="${f(A2[1])}" x2="${f(g1[0])}" y2="${f(g1[1])}" ${delay(d + 0.2)}/>` +
    `<line class="dr thin" pathLength="1" x1="${f(g2[0])}" y1="${f(g2[1])}" x2="${f(B2[0])}" y2="${f(B2[1])}" ${delay(d + 0.2)}/>` +
    `<polyline class="fade thin" points="${tick(A2, B2)}" ${delay(d + 0.4)}/><polyline class="fade thin" points="${tick(B2, A2)}" ${delay(d + 0.4)}/>` +
    `<text class="tx dimtx" text-anchor="middle" transform="translate(${f(mx)} ${f(my)}) rotate(${f(angle)}) translate(0 3.5)" ${delay(d + 0.5)}>${label}</text>` +
    '</g>'
  )
}

/** The numbered circle that leads to a detail view further down the page. */
const bubble = (n, at, d) =>
  `<a class="bub" [routerLink]="pageLink()" [fragment]="fragment(${n})" [class.on]="highlighted() === ${n}" tabindex="-1" aria-hidden="true" (mouseenter)="point(${n})" (mouseleave)="point(null)" ${delay(d)}>` +
  `<circle cx="${f(at[0])}" cy="${f(at[1])}" r="12"/><text x="${f(at[0])}" y="${f(at[1] + 4.2)}" text-anchor="middle">${n}</text></a>`

// Scene. Coordinates are world units; x runs to the right, y towards the viewer, z up.

// Clients: an npm or Docker client (back) and a browser (front).
push(box(-5.5, 19, 0, -1.1, 24, 2.2, { d: 0.35 }))
push(box(-5.5, 11.5, 0, -1.1, 16.5, 2.2, { d: 0.2 }))
// The browser's screen, drawn on its top face.
push(
  `<polyline class="dr thin" pathLength="1" points="${poly([P(-4.7, 20, 2.2), P(-1.9, 20, 2.2), P(-1.9, 23, 2.2), P(-4.7, 23, 2.2), P(-4.7, 20, 2.2)])}" ${delay(1.2)}/>`,
)
// A package on the client's top face: the archive it publishes or installs.
push(
  `<polyline class="dr thin" pathLength="1" points="${poly([P(-4.4, 12.6, 2.2), P(-2.2, 12.6, 2.2), P(-2.2, 15.4, 2.2), P(-4.4, 15.4, 2.2), P(-4.4, 12.6, 2.2)])}" ${delay(1.1)}/>`,
)

// The binary: a slab, the four layers on it, and the two registry protocols sitting on the api.
push(box(8, 0, 0, 38, 21.5, 1, { d: 0.7, cls: 'slab' }))
push(
  `<polygon class="dr thin" pathLength="1" points="${poly([P(8.8, 0.8, 1), P(37.2, 0.8, 1), P(37.2, 20.7, 1), P(8.8, 20.7, 1)])}" ${delay(1.9)}/>`,
)
push(box(10.5, 3, 1, 16.5, 15, 4, { d: 1.1 }))
// npm behind, docker in front and lower, so both names show on their front faces.
push(box(11, 3.6, 4, 16, 6.2, 6.6, { d: 1.3 }))
push(box(11, 6.6, 4, 16, 9.2, 5.6, { d: 1.35 }))
push(box(18.5, 3, 1, 27.5, 15, 3, { d: 1.4 }))
push(box(20.5, 6, 3, 25.5, 12, 7.2, { d: 1.65, cls: 'core' }))
push(box(29.5, 3, 1, 36, 15, 4, { d: 1.9 }))

// Crate names are identifiers, not translated.
push(faceText(11.3, 15, 1.3, 'api', 2.4, 'tx crate'))
push(faceText(11.5, 6.2, 5.75, 'npm', 2.45, 'tx crate small'))
push(faceText(11.5, 9.2, 4.3, 'docker', 2.5, 'tx crate small'))
push(faceText(19.2, 15, 1.0, 'application', 2.5, 'tx crate'))
push(faceText(21.1, 12, 3.45, 'domain', 2.6, 'tx crate'))
push(faceText(30.2, 15, 1.3, 'infrastructure', 2.7, 'tx crate'))

// Details drawn on the top faces: two ports on api, one court on domain, adapter plugs on infrastructure.
const quad = (x0, y0, x1, y1, z, d) =>
  `<polygon class="fade thin" points="${poly([P(x0, y0, z), P(x1, y0, z), P(x1, y1, z), P(x0, y1, z)])}" ${delay(d)}/>`
for (let i = 0; i < 2; i++) {
  const y = 10.2 + i * 2.4
  push(quad(11.4, y, 15.6, y + 1.6, 4, 2.4 + i * 0.1))
}
push(quad(21.5, 7, 24.5, 11, 7.2, 2.7))
for (let i = 0; i < 4; i++) {
  const y = 4.2 + i * 2.7
  push(quad(30.4, y, 32.6, y + 1.7, 4, 2.5 + i * 0.1))
  push(quad(33.4, y, 35.2, y + 1.7, 4, 2.5 + i * 0.1))
}

// Data: PostgreSQL and a stack of three drawers for the npm archives and the Docker blobs.
push(cylinder(43.5, 4.5, 3.2, 0, 4.2, 1.5))
for (let i = 0; i < 3; i++)
  push(box(41, 10.5, i * 1.5, 48.5, 16.5, i * 1.5 + 1, { d: 1.7 + i * 0.12 }))
for (let i = 0; i < 3; i++) {
  push(
    `<polyline class="fade thin" points="${poly([P(42, 16.5, i * 1.5 + 0.5), P(45.5, 16.5, i * 1.5 + 0.5)])}" ${delay(2.6)}/>`,
  )
}

// Upstream registries, which the proxies pull from: a base and three registries on it.
push(box(27, 27.5, 0, 39, 35.5, 0.8, { d: 2 }))
for (let i = 0; i < 3; i++) {
  push(
    box(28.4 + i * 3.4, 29.2, 0.8, 30.9 + i * 3.4, 33.8, 2.6 - i * 0.4, {
      d: 2.3 + i * 0.12,
      cls: i === 0 ? 'core' : 'cont',
    }),
  )
}
// Directories and mail, outside the instance: an area of three services.
push(
  `<polygon class="fade dash ln k8" points="${poly([P(9, 27.5, 0), P(22.5, 27.5, 0), P(22.5, 37.2, 0), P(9, 37.2, 0)])}" ${delay(2.3)}/>`,
)
;[
  [10.4, 30.2],
  [14.6, 30.2],
  [18.8, 30.2],
].forEach(([x, y], i) =>
  push(box(x, y, 0, x + 2.6, y + 2.6, 2.2, { d: 2.6 + i * 0.12, cls: 'pod' })),
)

// Links between the parts (ink): browser to api, infrastructure to its stores, and the directories asked at sign-in.
const LINK = 2.3
push(
  route(
    [
      [-1.1, 20.4, 1.05],
      [14, 20.4, 1.05],
      [14, 15.2, 1.05],
    ],
    { d: LINK },
  ),
)
push(
  route(
    [
      [36, 6, 2.4],
      [39.6, 6, 2.4],
    ],
    { d: LINK + 0.2 },
  ),
)
push(
  route(
    [
      [36, 12.5, 2.0],
      [40.6, 12.5, 2.0],
    ],
    { d: LINK + 0.3 },
  ),
)
push(
  route(
    [
      [16, 21.5, 0.3],
      [16, 29.4, 0.3],
    ],
    { d: LINK + 0.5, dashed: true },
  ),
)

// The flow (orange): an install reaches the api, goes through a group to its proxy, which pulls from upstream. The
// lines are drawn once, then a pulse of light runs along them twice and stops.
const FLOW = 3.4
const mainPath = [
  [-1.1, 16.5, 1.05],
  [33, 16.5, 1.05],
]
const toUpstream = [
  [33, 16.5, 1.05],
  [33, 21.5, 1.05],
  [33, 21.5, 0.1],
  [33, 26.8, 0.1],
]
function flowSegment(pts, d, duration, arrow) {
  const s = project(pts)
  let html = `<polyline class="dr acc" pathLength="1" points="${poly(s)}" style="--d:${f(d)}s;--t:${duration}s"/>`
  if (arrow)
    html += `<polygon class="ahead acc" points="${arrowHead(s[s.length - 2], s[s.length - 1], 9)}" style="--d:${f(d + duration - 0.1)}s"/>`
  return html
}
const pulse = (pts, d) =>
  `<polyline class="pulse" pathLength="1" points="${poly(project(pts))}" style="--d:${f(d)}s"/>`
push(
  `<g class="flow">${flowSegment(mainPath, FLOW, 1.7, false)}${flowSegment(toUpstream, FLOW + 1.6, 0.8, true)}</g>`,
)
push(`<g class="pulses">${pulse(mainPath, FLOW + 2.7)}${pulse(toUpstream, FLOW + 4.8)}</g>`)
;[13.5, 23, 32.7].forEach((x, i) => {
  const p = P(x, 16.5, 1.05)
  push(
    `<circle class="dot big" cx="${f(p[0])}" cy="${f(p[1])}" r="3.6" style="--d:${f(FLOW + 0.5 + i * 0.5)}s"/>`,
  )
})

// Dimension line.
push(dimension([8, 0, 0], [38, 0, 0], [0, -3.2], t('plan.dimBinary'), 4, 205))

// Annotations: a leader from a point of the drawing to a block of text, offset in screen space. The first line is the
// name, in bold; the others are the description.
const at = (p, dx, dy) => [p[0] + dx, p[1] + dy]
function note(anchor, offset, rows, d, side = 'start') {
  const to = at(anchor, ...offset)
  let html = leader(anchor, to, d)
  const x = to[0] + (side === 'end' ? -6 : side === 'middle' ? 0 : 6)
  rows.forEach((row, i) => {
    html += text(
      x,
      to[1] + 4 + i * 16,
      row,
      d + 0.3 + i * 0.05,
      i === 0 ? 'tx lbl' : 'tx note',
      side,
    )
  })
  return html
}
/** The description lines of a part: plan.<id>.l1 to l<count>. */
const description = (id, count) =>
  Array.from({ length: count }, (_, i) => t(`plan.${id}.l${i + 1}`))
push(
  note(
    P(-4.5, 14, 2.2),
    [-70, -60],
    [t('plan.client.name'), ...description('client', 2)],
    3.0,
    'middle',
  ),
)
push(note(P(-3, 24, 0.6), [24, 62], [t('plan.browser.name'), ...description('browser', 2)], 3.0))
push(note(P(12, 3.6, 6.6), [-20, -56], ['api, npm, docker', ...description('api', 2)], 3.1, 'end'))
push(note(P(23, 6.5, 7.2), [-4, -72], ['domain', ...description('domain', 1)], 3.15))
push(note(P(26.5, 3, 3), [44, -64], ['application', ...description('application', 1)], 3.2))
push(note(P(35.5, 3, 4), [40, -72], ['infrastructure', ...description('infrastructure', 3)], 3.25))
push(note(P(46.5, 4.5, 2.5), [18, -10], ['PostgreSQL', ...description('postgres', 4)], 3.3))
push(note(P(48.5, 14, 0.5), [26, 14], [t('plan.storage.name'), ...description('storage', 2)], 3.3))
push(
  note(P(37.5, 31.5, 1.2), [40, 6], [t('plan.upstream.name'), ...description('upstream', 3)], 3.4),
)
push(
  note(
    P(15, 37.2, 0),
    [-10, 30],
    [t('plan.identity.name'), ...description('identity', 1)],
    3.4,
    'middle',
  ),
)

// Labels of the flow.
const requestAt = P(2, 16.5, 1.05)
push(text(requestAt[0] + 6, requestAt[1] + 24, t('plan.flow.request'), 4.4, 'tx acct', 'middle'))
const groupAt = P(19, 18.6, 1.0)
push(text(groupAt[0] - 34, groupAt[1] + 26, t('plan.flow.group'), 4.8, 'tx acct', 'middle'))
const upstreamAt = P(33, 23, 0)
push(text(upstreamAt[0] + 12, upstreamAt[1] + 4, t('plan.flow.upstream'), 5, 'tx acct'))

// The seven numbered circles, which link to the numbered tiles of the home page:
// 1 rights, 2 repository types, 3 package page, 4 image scan, 5 search, 6 organisations, 7 quota and retention.
;[
  [1, P(10.5, 10, 2.5), [-42, -14]],
  [2, P(24, 16.5, 1.05), [2, 36]],
  [3, P(-2.5, 23.2, 1), [40, 14]],
  [4, P(35.2, 12.5, 4), [30, -34]],
  [5, P(12, 20.4, 1.05), [14, -30]],
  [6, P(-5.5, 14, 1.1), [-44, 18]],
  [7, P(48.5, 12.5, 1.2), [22, 6]],
].forEach(([n, p, off]) => {
  const to = at(p, ...off)
  const d = 4.5 + n * 0.1
  push(
    `<polyline class="fade ld" points="${pt(p)} ${pt(to)}" ${delay(d)}/><circle class="dot" cx="${f(p[0])}" cy="${f(p[1])}" r="2" ${delay(d)}/>`,
  )
  push(bubble(n, to, d + 0.1))
})

const svg = `<svg class="plan" viewBox="-420 -40 1150 640" role="img" aria-labelledby="plan-title plan-desc" focusable="false">
<title id="plan-title">${t('plan.title')}</title>
<desc id="plan-desc">${t('plan.description')}</desc>
<defs>
<pattern id="plan-hl" width="5" height="5" patternUnits="userSpaceOnUse" patternTransform="rotate(60)"><line x1="0" y1="0" x2="0" y2="5" class="hatchline"/></pattern>
<pattern id="plan-hr" width="3.4" height="3.4" patternUnits="userSpaceOnUse" patternTransform="rotate(-60)"><line x1="0" y1="0" x2="0" y2="3.4" class="hatchline"/></pattern>
</defs>
${out.join('\n')}
</svg>
`

const config = (await prettier.resolveConfig(output)) ?? {}
const formatted = await prettier.format(
  `<!-- Generated by scripts/build-plan.mjs (npm run plan). Edit the script, not this file. -->\n${svg}`,
  { ...config, parser: 'angular' },
)
writeFileSync(output, formatted)
console.log(`plan: ${output} written (${formatted.length} bytes)`)
