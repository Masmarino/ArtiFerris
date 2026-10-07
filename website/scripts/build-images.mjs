// Regenerates the raster assets drawn from the ArtiFerris crab: logos, favicon, Apple touch icon and the 1200x630 social
// preview. They are committed; run `npm run images` after changing the logo or the preview text.
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import opentype from 'opentype.js'
import sharp from 'sharp'
import { readSite, root } from './paths.mjs'

const images = join(root, 'public/images')
// The 600px crab of frontend/public/Logo.png, unchanged.
const source = readFileSync(join(root, 'scripts/assets/artiferris.png'))

const transparent = { r: 0, g: 0, b: 0, alpha: 0 }

// Trim the transparent margin so the crab fills the same share of every size.
const crab = await sharp(source).trim().png().toBuffer()

const fitted = (size) => sharp(crab).resize(size, size, { fit: 'contain', background: transparent })

async function square(size, file, { background, padding = 0 } = {}) {
  const logo = await fitted(size - padding * 2)
    .png()
    .toBuffer()
  await sharp({
    create: { width: size, height: size, channels: 4, background: background ?? transparent },
  })
    .composite([{ input: logo, left: padding, top: padding }])
    .png({ compressionLevel: 9 })
    .toFile(file)
}

await square(64, join(images, 'logo-64.png'))
await square(128, join(images, 'logo-128.png'))

// favicon.ico: three small PNGs in an ICO container, 6 KB instead of the 295 KB of the 586px icon.
const iconSizes = [16, 32, 48]
const iconPngs = await Promise.all(
  iconSizes.map((size) => fitted(size).png({ compressionLevel: 9 }).toBuffer()),
)
const header = Buffer.alloc(6 + 16 * iconPngs.length)
header.writeUInt16LE(1, 2) // type: icon
header.writeUInt16LE(iconPngs.length, 4)
let offset = header.length
iconPngs.forEach((png, index) => {
  const entry = 6 + 16 * index
  header.writeUInt8(iconSizes[index], entry)
  header.writeUInt8(iconSizes[index], entry + 1)
  header.writeUInt16LE(1, entry + 4) // colour planes
  header.writeUInt16LE(32, entry + 6) // bits per pixel
  header.writeUInt32LE(png.length, entry + 8)
  header.writeUInt32LE(offset, entry + 12)
  offset += png.length
})
writeFileSync(join(root, 'public/favicon.ico'), Buffer.concat([header, ...iconPngs]))

await square(180, join(root, 'public/apple-touch-icon.png'), {
  background: { r: 255, g: 255, b: 255, alpha: 1 },
  padding: 22,
})

// The icons of site.webmanifest, for Android and an installed Chrome: two on a transparent ground, and a maskable one
// that Android crops to its own shape, so the crab keeps inside the central circle of radius 40 %, on white like the
// Apple touch icon.
mkdirSync(join(root, 'public/icons'), { recursive: true })
await square(192, join(root, 'public/icons/icon-192.png'))
await square(512, join(root, 'public/icons/icon-512.png'))
await square(512, join(root, 'public/icons/icon-maskable-512.png'), {
  background: { r: 255, g: 255, b: 255, alpha: 1 },
  padding: 111,
})

// Social previews, one per language: the title (from the site's own translations), a line on what the product covers
// and a crop of the home page's architecture plan, in the colours of the dark theme. The text is drawn as outlines
// from scripts/assets/fonts, because sharp cannot load custom fonts. The drawing is the template that
// `npm run plan` generates, minus its labels, animation hooks and Angular bindings.
const W = 1200
const H = 630
const sans = opentype.parse(
  readFileSync(join(root, 'scripts/assets/fonts/ibm-plex-sans-condensed-600.ttf')).buffer,
)
const mono = opentype.parse(
  readFileSync(join(root, 'scripts/assets/fonts/ibm-plex-mono-400.ttf')).buffer,
)

const COMMAND_POINTS = {
  M: ['x', 'y'],
  L: ['x', 'y'],
  Q: ['x1', 'y1', 'x', 'y'],
  C: ['x1', 'y1', 'x2', 'y2', 'x', 'y'],
}
const round2 = (value) => String(Math.round(value * 100) / 100)

/** One line of text as an SVG path. `tracking` is extra spacing between letters, in em. */
const line = (font, text, x, baseline, size, fill, tracking = 0) => {
  const path = font.getPath(text, x, baseline, size, { letterSpacing: tracking })
  // opentype.js's toPathData writes "NaN" for some very small numbers, hence the hand-written path data.
  const data = path.commands
    .map((c) =>
      c.type === 'Z' ? 'Z' : c.type + COMMAND_POINTS[c.type].map((key) => round2(c[key])).join(' '),
    )
    .join('')
  return `<path d="${data}" fill="${fill}"/>`
}

// Dark theme colours of the site, on a plain ground.
const paper = '#0c1015'
const ink = '#e7ebf1'
const muted = '#aab3c0'
const orange = '#f0622a'
const face = '#141a21'
const top = '#1a212a'
const core = '#3a2a2e'
const slab = '#10151b'

const template = readFileSync(
  join(root, 'src/app/shared/architecture-plan/architecture-plan.html'),
  'utf-8',
)
// Strip what the card does not show: text, numbered links, leaders, dimension lines and the animation hooks
// (pathLength, --d delays); the pulses are hidden.
const drawing = template
  .replace(/<!--[\s\S]*?-->/g, '')
  .replace(/<svg[^>]*>/, '')
  .replace(/<\/svg>/, '')
  .replace(/<(title|desc|text)\b[\s\S]*?<\/\1>/g, '')
  .replace(/<a\b[\s\S]*?<\/a>/g, '')
  .replace(/<polyline\s+class="(?:dr|fade) ld"[^>]*\/>/g, '')
  .replace(/<circle\s+class="dot"[^>]*\/>/g, '')
  .replace(/<g\s+class="part dim">[\s\S]*?<\/g>/g, '')
  .replace(/\s(?:pathLength|style)="[^"]*"/g, '')
  .replace(/\sclass="(?:[^"]*\s)?(?:fade|pulse)(?:\s[^"]*)?"/g, (match) =>
    match.includes('pulse') ? ' display="none"' : match,
  )
const planStyle = `
  .ln { fill: none; stroke: ${ink}; stroke-width: 1.7; stroke-linejoin: round; stroke-linecap: round }
  .f { fill: ${face} } .top { fill: ${top} } .core .top { fill: ${core} } .slab > .f { fill: ${slab} }
  .thin, .ld { fill: none; stroke: ${muted}; stroke-width: 1.1 }
  .hat { stroke: none } .hl { fill: url(#plan-hl) } .hr { fill: url(#plan-hr) }
  .hatchline { stroke: ${ink}; stroke-width: .8; opacity: .3 }
  .dash { stroke-dasharray: 5 4 } .ghost { opacity: .45 }
  .acc { fill: none; stroke: ${orange}; stroke-width: 3.4; stroke-linecap: round; stroke-linejoin: round }
  .ahead { fill: ${orange}; stroke: none } .ahead.ln { fill: ${ink} }
  .dot { fill: ${ink} } .dot.big { fill: ${orange}; stroke: ${paper}; stroke-width: 1.5 }`

// The plan sits at the right and runs off the card; the crop starts at the Git client.
const crop = { x: -420, y: -20, width: 1150, height: 660 }
const scale = 0.6
const planX = 534
const planY = 160
const gridColumns = Array.from({ length: Math.ceil(W / 24) }, (_, i) => `M${i * 24} 0V${H}`).join(
  '',
)
const gridRows = Array.from({ length: Math.ceil(H / 24) }, (_, i) => `M0 ${i * 24}H${W}`).join('')

const readJson = (path) => JSON.parse(readFileSync(join(root, path), 'utf-8'))
const taglines = readJson('scripts/assets/og-text.json')
const { langs } = readSite()

/** Splits `text` into lines no wider than `maxWidth` at `size`, or returns null if a single word is too wide. */
function wrap(text, size, maxWidth, tracking) {
  const width = (value) => sans.getAdvanceWidth(value, size, { letterSpacing: tracking })
  const lines = []
  let current = ''
  for (const word of text.split(' ')) {
    const next = current ? `${current} ${word}` : word
    if (width(next) <= maxWidth || !current) {
      current = next
    } else {
      lines.push(current)
      current = word
    }
  }
  lines.push(current)
  return lines.every((value) => width(value) <= maxWidth) ? lines : null
}

/** The largest title size that fits on at most four lines: the languages differ a lot in length. */
function fitTitle(text) {
  for (let size = 76; size >= 44; size -= 2) {
    const lines = wrap(text, size, 500, -0.01)
    if (lines && lines.length <= 4) return { size, lines }
  }
  throw new Error(`The title does not fit on four lines: ${text}`)
}

const logo = await fitted(64).png().toBuffer()

for (const lang of langs) {
  const { size, lines } = fitTitle(readJson(`src/app/i18n/${lang}.json`).home.hero.title)
  const lineHeight = Math.round(size * 1.1)
  const firstBaseline = lines.length > 3 ? 196 : 232
  const lastBaseline = firstBaseline + (lines.length - 1) * lineHeight
  const ruleY = lastBaseline + 36
  const tagline = wrap(taglines[lang], 25, 470, 0)
  if (!tagline || tagline.length > 2) throw new Error(`The tagline does not fit: ${taglines[lang]}`)

  const svg = `
<svg xmlns="http://www.w3.org/2000/svg" width="${W}" height="${H}" viewBox="0 0 ${W} ${H}">
  <rect width="${W}" height="${H}" fill="${paper}"/>
  <path d="${gridColumns}${gridRows}" stroke="${ink}" stroke-opacity=".05" stroke-width="1" fill="none"/>
  <svg x="${planX}" y="${planY}" width="${crop.width * scale}" height="${crop.height * scale}" viewBox="${crop.x} ${crop.y} ${crop.width} ${crop.height}" overflow="hidden">
    <style>${planStyle}</style>
    ${drawing}
  </svg>
  ${lines.map((text, index) => line(sans, text, 64, firstBaseline + index * lineHeight, size, ink, -0.01)).join('')}
  <rect x="64" y="${ruleY}" width="120" height="6" fill="${orange}"/>
  ${tagline.map((text, index) => line(sans, text, 64, ruleY + 52 + index * 34, 25, muted)).join('')}
  ${line(mono, 'www.artiferris.pro', 64, H - 48, 22, muted)}
  ${line(sans, 'ArtiFerris', 134, 92, 42, ink, 0)}
</svg>`

  await sharp(Buffer.from(svg))
    .composite([{ input: logo, left: 64, top: 52 }])
    .png({ compressionLevel: 9, palette: true, quality: 92 })
    .toFile(join(images, `og-${lang}.png`))
}

console.log(
  `images: logo-64, logo-128, favicon.ico, apple-touch-icon, manifest icons, og-${langs.join(', og-')} written`,
)
