#!/usr/bin/env node
// Rebuilds the product screenshots in public/images/screens. Per screen and theme there are two files:
//   <name>-<theme>.webp         the whole screen, 1600x1000, linked from its detail;
//   <name>-detail-<theme>.webp  a crop of its most telling part at twice the pixel density, so that the interface text
//                               stays legible at 560 px or more.
//
// The shots are renders of the application's own components with the data of its Storybook stories, driven through
// Chrome, so a screenshot always matches the commit it was built from.
//
// 1. Build the application's Storybook and serve it (from the repository root):
//      cd frontend && npm run build-storybook                               # writes frontend/storybook-static
//      python3 -m http.server 6020 --directory frontend/storybook-static  # any static server will do
// 2. Run it, with Google Chrome installed (CHROME_PATH if it is not in a usual place):
//      node website/scripts/capture-screens.mjs [--storybook http://localhost:6020] [--only repository,docs] [--detail-only] [--out <dir>]
//    --only takes names from SHOTS below; --detail-only skips the whole screens; --out defaults to public/images/screens.
//
// puppeteer-core and sharp are installed on first run into $TMPDIR/artiferris-screens-deps, so that the site does not
// carry a browser driver in its own dependencies.
//
// A story renders one component, but the application shows pages inside a frame (the signed-in shell or the public
// layout). So each shot opens two stories, the page and a frame, and moves the page into the frame where the router
// outlet would be, component styles included. Two things are patched afterwards: the active menu entry (stories run
// without a router) and the names in the demo data, so no real person appears (TEXT_REPLACEMENTS). HIDE removes
// fixture cards that only exist to test edge cases.
// The dark variant sets data-theme="dark" like the theme switch and emulates prefers-color-scheme: dark, which the
// logo and the code highlighting follow.

import { spawnSync } from 'node:child_process'
import { existsSync, mkdirSync, statSync, writeFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { root } from './paths.mjs'

const WIDTH = 1600
const HEIGHT = 1000
const WHOLE = { x: 0, y: 0, width: WIDTH, height: HEIGHT }
const WEBP_QUALITY = 86
const MAX_BYTES = 250_000
const DETAIL_MAX_BYTES = 140_000

// Frame stories: the signed-in shell and the public layout.
const SHELL = 'shell-appshell--default'
const PUBLIC = 'public-publiclayout--default'

// name -> the frame story, the story of the page, the element of the frame that receives it (`slot`), what the router
// would set (`active`, the menu entry marked as current), and the window and clip of the detail shot. `full` lets the
// page use the whole width of the public layout.
const SHOTS = {
  repositories: {
    detail: { width: 1240, height: 820, clip: { x: 264, y: 84, width: 930, height: 620 } },
    frame: SHELL,
    slot: 'main.gbt-app-shell__content',
    page: 'repositories-repositorydetail--proxy-repository',
    active: 'Dépôts',
  },
  package: {
    detail: { width: 1240, height: 820, clip: { x: 264, y: 130, width: 930, height: 665 } },
    frame: SHELL,
    slot: 'main.gbt-app-shell__content',
    page: 'repositories-packagedetailpage--npm-package-with-readme',
    active: 'Dépôts',
  },
  scan: {
    detail: { width: 1240, height: 1100, clip: { x: 264, y: 420, width: 930, height: 540 } },
    frame: SHELL,
    slot: 'main.gbt-app-shell__content',
    page: 'repositories-packagedetailpage--docker-image-with-vulnerabilities',
    active: 'Dépôts',
  },
  organizations: {
    detail: { width: 1240, height: 820, clip: { x: 232, y: 84, width: 960, height: 560 } },
    frame: SHELL,
    slot: 'main.gbt-app-shell__content',
    page: 'admin-organizationspage--super-admin-with-organization-selected',
    active: 'Organisations',
  },
  // These two pages render their own public layout, so they need no frame.
  explore: {
    detail: { width: 1240, height: 820, clip: { x: 150, y: 84, width: 940, height: 620 } },
    frame: null,
    page: 'public-catalog-explorerpage--default',
  },
  docs: {
    detail: { width: 1240, height: 820, clip: { x: 0, y: 64, width: 960, height: 640 } },
    frame: null,
    page: 'docs-documentation--public-page',
  },
}

// The demo data uses the maintainer's own names. Replaced in text nodes only, never in attributes or URLs; longer
// strings first.
const TEXT_REPLACEMENTS = [
  ['Florian Simon', 'Camille Durand'],
  ['florian', 'camille'],
  ['masmarino', 'acme'],
  ['hangar-demo', 'acme-demo'],
]
// The stories build registry addresses from the address they are served on: an organization's repository moves to its
// subdomain, anything else to the application's host.
const HOST_REPLACEMENTS = [
  ['http(?:s)?://localhost:\\d+/o/acme/', 'https://acme.artiferris.pro/'],
  ['localhost:\\d+/o/acme/', 'acme.artiferris.pro/'],
  ['http(?:s)?://localhost:\\d+', 'https://app.artiferris.pro'],
  ['localhost:\\d+', 'app.artiferris.pro'],
]
const INITIALS_REPLACEMENTS = { FS: 'CD', FL: 'CA', MD: 'ML' }

// Fixture cards that only exist to test line wrapping and make poor product shots.
const HIDE = []

function parseArgs(argv) {
  const args = {
    storybook: 'http://localhost:6020',
    only: null,
    detailOnly: false,
    out: join(root, 'public/images/screens'),
  }
  for (let i = 0; i < argv.length; i++) {
    const [key, value] = [argv[i], argv[i + 1]]
    if (key === '--storybook') ((args.storybook = value), i++)
    else if (key === '--only') ((args.only = value.split(',')), i++)
    else if (key === '--detail-only') args.detailOnly = true
    else if (key === '--out') ((args.out = resolve(value)), i++)
    else throw new Error(`Unknown argument: ${key}`)
  }
  args.storybook = args.storybook.replace(/\/$/, '')
  return args
}

function loadDependencies() {
  const dir = join(tmpdir(), 'artiferris-screens-deps')
  mkdirSync(dir, { recursive: true })
  const require = createRequire(join(dir, 'package.json'))
  try {
    return { puppeteer: require('puppeteer-core'), sharp: require('sharp') }
  } catch {
    console.log(`Installing puppeteer-core and sharp into ${dir}`)
    writeFileSync(join(dir, 'package.json'), '{"name":"artiferris-screens-deps","private":true}\n')
    const result = spawnSync(
      'npm',
      ['install', '--no-audit', '--no-fund', 'puppeteer-core', 'sharp'],
      { cwd: dir, stdio: 'inherit' },
    )
    if (result.status !== 0) throw new Error('npm install failed')
    return { puppeteer: require('puppeteer-core'), sharp: require('sharp') }
  }
}

function findChrome() {
  const candidates = [
    process.env.CHROME_PATH,
    '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
    '/usr/bin/google-chrome',
    '/usr/bin/google-chrome-stable',
    '/usr/bin/chromium',
  ]
  const found = candidates.find((path) => path && existsSync(path))
  if (!found) throw new Error('Chrome not found: set CHROME_PATH')
  return found
}

// Opens a story without Storybook's chrome and waits for it to render and settle (fonts, icons, play function).
async function openStory(browser, base, id, dark, view) {
  const page = await browser.newPage()
  await page.setViewport({ width: view.width, height: view.height, deviceScaleFactor: view.scale })
  await page.emulateMediaFeatures([
    { name: 'prefers-color-scheme', value: dark ? 'dark' : 'light' },
  ])
  await page.goto(`${base}/iframe.html?id=${id}&viewMode=story`, { waitUntil: 'networkidle0' })
  await page.waitForFunction(
    () =>
      document.body.classList.contains('sb-show-main') &&
      !document.body.classList.contains('sb-show-errordisplay'),
    { timeout: 30000 },
  )
  await page.evaluate((isDark) => {
    // Stories do not set a theme themselves, so force it on every one.
    if (isDark) document.documentElement.setAttribute('data-theme', 'dark')
    else document.documentElement.removeAttribute('data-theme')
    return document.fonts.ready
  }, dark)
  await new Promise((done) => setTimeout(done, 2500))
  return page
}

// The markup of the story's host element and the component styles Angular added to <head>.
function extractPage(page) {
  return page.evaluate(() => {
    // A field's value is a property, which innerHTML leaves out: write it into the markup first.
    for (const input of document.querySelectorAll('input')) input.setAttribute('value', input.value)
    for (const option of document.querySelectorAll('option'))
      option.toggleAttribute('selected', option.selected)
    return {
      html: document.querySelector('#storybook-root').firstElementChild.innerHTML,
      styles: [...document.querySelectorAll('head style')].map((style) => style.textContent),
    }
  })
}

// Runs in the frame page. Puppeteer serialises it, so it cannot use anything defined outside its own body.
function compose(extracted, shot, texts) {
  // A story is shown padded, and a play function can leave the focus ring on a control.
  document.body.classList.remove('sb-main-padded')
  document.activeElement?.blur?.()
  const slot = shot.slot ? document.querySelector(shot.slot) : document.body

  if (extracted) {
    const known = new Set(
      [...document.head.querySelectorAll('style')].map((style) => style.textContent),
    )
    for (const css of extracted.styles) {
      if (known.has(css)) continue
      const style = document.createElement('style')
      style.textContent = css
      document.head.append(style)
    }
    slot.innerHTML = extracted.html
    if (shot.full) slot.classList.add('public-layout__content--full')
  }

  // The menu is styled from aria-current, which the router normally sets.
  if (shot.active) {
    for (const link of document.querySelectorAll('nav a, header a')) {
      if (link.getAttribute('aria-current') === 'page') link.removeAttribute('aria-current')
      if (link.textContent.trim() === shot.active) link.setAttribute('aria-current', 'page')
    }
  }
  const current = document.querySelector('.gbt-breadcrumb__current')
  if (current && shot.crumb) current.textContent = shot.crumb

  for (const { shot: name, selector, text } of texts.hide) {
    if (name !== shot.name) continue
    for (const item of slot.querySelectorAll(selector))
      if (item.textContent.includes(text)) item.remove()
  }

  const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT)
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    const trimmed = node.textContent.trim()
    if (trimmed in texts.initials)
      node.textContent = node.textContent.replace(trimmed, texts.initials[trimmed])
    else {
      for (const [from, to] of texts.replacements)
        node.textContent = node.textContent.split(from).join(to)
      for (const [pattern, to] of texts.hosts)
        node.textContent = node.textContent.replace(new RegExp(pattern, 'g'), to)
    }
  }
}

// `view` is the window ({ width, height, scale }) and `clip` the part of it to keep, in CSS pixels. A page taller than
// the window is cut at it: the shot keeps its top.
async function capture(browser, base, name, shot, dark, view, clip) {
  shot = { ...shot, name }
  let extracted = null
  if (shot.frame) {
    const content = await openStory(browser, base, shot.page, dark, view)
    extracted = await extractPage(content)
    await content.close()
  }

  const page = await openStory(browser, base, shot.frame ?? shot.page, dark, view)
  await page.evaluate(compose, extracted, shot, {
    replacements: TEXT_REPLACEMENTS,
    hosts: HOST_REPLACEMENTS,
    initials: INITIALS_REPLACEMENTS,
    hide: HIDE,
    dark,
  })
  await new Promise((done) => setTimeout(done, 800))

  const png = await page.screenshot({ type: 'png', clip })
  await page.close()
  return png
}

async function toWebp(sharp, png, maxBytes = MAX_BYTES) {
  let quality = WEBP_QUALITY
  for (;;) {
    const buffer = await sharp(png).webp({ quality, effort: 6 }).toBuffer()
    if (buffer.length <= maxBytes || quality <= 60) return { buffer, quality }
    quality -= 4
  }
}

async function main() {
  const args = parseArgs(process.argv.slice(2))
  const { puppeteer, sharp } = loadDependencies()
  const names = args.only ?? Object.keys(SHOTS)
  for (const name of names)
    if (!SHOTS[name])
      throw new Error(`Unknown shot "${name}". Known: ${Object.keys(SHOTS).join(', ')}`)

  mkdirSync(args.out, { recursive: true })
  const browser = await puppeteer.launch({
    executablePath: findChrome(),
    headless: true,
    args: ['--hide-scrollbars'],
  })
  try {
    for (const name of names) {
      for (const theme of ['light', 'dark']) {
        const dark = theme === 'dark'
        const shot = SHOTS[name]
        const outputs = []
        if (!args.detailOnly) {
          const view = { width: WIDTH, height: HEIGHT, scale: 1 }
          const png = await capture(browser, args.storybook, name, shot, dark, view, WHOLE)
          outputs.push([`${name}-${theme}.webp`, await toWebp(sharp, png)])
        }
        const { width, height, clip } = shot.detail
        const view = { width, height, scale: 2 }
        const png = await capture(browser, args.storybook, name, shot, dark, view, clip)
        outputs.push([`${name}-detail-${theme}.webp`, await toWebp(sharp, png, DETAIL_MAX_BYTES)])
        for (const [fileName, { buffer, quality }] of outputs) {
          const file = join(args.out, fileName)
          writeFileSync(file, buffer)
          console.log(`${file}  ${(statSync(file).size / 1024).toFixed(0)} KB  (q${quality})`)
        }
      }
    }
  } finally {
    await browser.close()
  }
}

main().catch((error) => {
  console.error(error)
  process.exit(1)
})
