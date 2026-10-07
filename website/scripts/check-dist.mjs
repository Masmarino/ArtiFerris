// Checks the prerendered site after `npm run build`, so that a build which lost a page, a language attribute, a demo or
// some SEO markup fails instead of shipping. The counts below follow the templates: change them together.
import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs'
import { join } from 'node:path'
import { dist, readSite } from './paths.mjs'
import { pageUrl } from './sitemap.mjs'

const site = readSite()

const errors = []
const fail = (message) => errors.push(message)

function read(file) {
  const path = join(dist, file)
  if (!existsSync(path)) {
    fail(`missing ${file}`)
    return null
  }
  return readFileSync(path, 'utf-8')
}

const count = (html, pattern) => (html.match(pattern) ?? []).length

// The architecture plan, on the home page: one labelled image with real text, seven numbered circles and the flow of a
// install. `fragments` are the tiles its circles link to.
function checkPlan(file, html, fragments) {
  if (!html.includes('<app-architecture-plan')) fail(`${file}: the architecture plan is missing`)
  const plan = html.match(/<svg[^>]*\bclass="plan"[^>]*>/)?.[0] ?? ''
  if (!plan.includes('role="img"') || !plan.includes('aria-labelledby="plan-title plan-desc"')) {
    fail(`${file}: the plan is not one labelled image`)
  }
  if (!/<desc[^>]*id="plan-desc"[^>]*>[^<]{80,}/.test(html))
    fail(`${file}: the plan has no text description`)
  if (count(html, /<text[\s>]/g) < 40) fail(`${file}: the plan has too few text labels`)
  if (count(html, /class="bub"/g) !== 7) fail(`${file}: the plan should have seven circles`)
  if (count(html, /<pattern[^>]*id="plan-h/g) !== 2) fail(`${file}: the plan lost its hatching`)
  if (!html.includes('class="pulses"')) fail(`${file}: the plan lost the pulse of the install`)
  for (const fragment of new Set(fragments)) {
    if (!html.includes(`#${fragment}"`)) fail(`${file}: no link to #${fragment}`)
    if (!new RegExp(`id="${fragment}"`).test(html)) fail(`${file}: #${fragment} is missing`)
  }
  if (count(html, /class="fig__views"/g) !== 1)
    fail(`${file}: the keyboard list of the numbers is missing`)
}

let pages = 0
for (const [id, path] of Object.entries(site.pages)) {
  for (const lang of site.langs) {
    const file = path ? `${lang}/${path}/index.html` : `${lang}/index.html`
    const html = read(file)
    if (html === null) continue
    pages++
    if (!html.includes(`<html lang="${lang}"`)) fail(`${file}: <html lang> is not "${lang}"`)
    const h1 = count(html, /<h1[\s>]/g)
    if (h1 !== 1) fail(`${file}: ${h1} <h1> elements, expected 1`)
    if (!html.includes(`<link rel="canonical" href="${pageUrl(site, lang, path)}"`))
      fail(`${file}: wrong canonical`)
    const hreflangs = count(html, /<link rel="alternate" hreflang="/g)
    if (hreflangs !== site.langs.length + 1) fail(`${file}: ${hreflangs} hreflang links`)
    if (!html.includes(`<meta property="og:image" content="${site.siteUrl}/images/og-${lang}.png"`))
      fail(`${file}: og:image is not the ${lang} preview`)
    if (!/<title>[^<]{10,}<\/title>/.test(html)) fail(`${file}: no title`)
    // The CSP allows no inline script, JSON data blocks aside.
    const inline = [...html.matchAll(/<script(?![^>]*\bsrc=)([^>]*)>/g)].filter(
      ([, attributes]) => !/type="application\/(?:json|ld\+json)"/.test(attributes),
    )
    if (inline.length > 0) fail(`${file}: ${inline.length} inline script(s)`)
    // theme-init.js adds the `js` class at runtime and the page must read fine without it, so none may be prerendered.
    if (/<html[^>]*class=/.test(html)) fail(`${file}: <html> has a class in the prerendered markup`)
    // A raw translation key in the text means a missing translation. File names and example hosts look like keys too.
    if (
      /\b(?:plan|home|footer|features|roadmap|nav|scene|caps|registries|screens|install)\.[a-z]+\.[a-z0-9]+\b(?![^<]*>)/i.test(
        html
          .replace(/<(?:script|style)[\s\S]*?<\/(?:script|style)>/g, '')
          .replace(/<[^>]*>/g, ' ')
          .replace(/[\w-]+\.(?:ya?ml|json|env|rs|git|npmrc)\b/g, '')
          .replace(/\b[\w-]+\.(?:artiferris\.pro|npmjs\.org)\S*/g, ''),
      )
    ) {
      fail(`${file}: a translation key is shown as text`)
    }
    if (!/class="footer__legal[ "]/.test(html)) fail(`${file}: the footer is missing`)
    if (id === 'features') {
      const panels = html.match(/class="tabs__panel[^"]*"/g) ?? []
      if (panels.length !== 6) fail(`${file}: ${panels.length} tab panels, expected 6`)
      if (
        panels.filter((panel) => panel.includes('is-active')).length !== 1 ||
        !panels[0]?.includes('is-active')
      ) {
        fail(`${file}: only the first panel should be active`)
      }
      if (count(html, /role="tab(?:list|panel)?"/g) > 0) {
        fail(`${file}: ARIA tab roles must be added by the scripts, not prerendered`)
      }
      if (count(html, /id="feature-/g) !== 10) fail(`${file}: feature anchors missing`)
      if (count(html, /<app-window/g) !== 6) fail(`${file}: six screenshot windows expected`)
      if (count(html, /class="shot"/g) !== 6)
        fail(`${file}: the six screenshots should link to the whole screen`)
      if (!/<img[^>]*screens\/repositories-detail-light\.webp/.test(html))
        fail(`${file}: the screenshots are not the tight crops`)
      if (count(html, /class="spec__row more__item"/g) !== 4) fail(`${file}: expected 4 more rows`)
      if (html.includes('<app-architecture-plan'))
        fail(`${file}: the plan belongs to the home page only`)
    }
    if (id === 'home') {
      // The hero claims a memory figure and prints the command output that backs it.
      if (!html.includes('kubectl top pod -n artiferris'))
        fail(`${file}: the measured output is missing`)
      if (!/class="proof__terminal"[\s\S]*?\d+Mi\b/.test(html))
        fail(`${file}: the measured output has no memory figure`)
      if (!html.includes('docker compose up -d --build')) fail(`${file}: hero command missing`)
      if (count(html, /<gbt-copy-button/g) < 1) fail(`${file}: the copy button is missing`)
      if (count(html, /class="spec__row why__row"/g) !== 4)
        fail(`${file}: four situations expected`)
      if (count(html, /class="spec__row limits__item"/g) !== 4)
        fail(`${file}: four limits expected`)
      if (count(html, /class="tile tile--/g) !== 8) fail(`${file}: eight tiles expected`)
      if (count(html, /class="strip__item"/g) !== 5) fail(`${file}: five roadmap items expected`)
      for (const section of ['why', 'product', 'architecture', 'roadmap']) {
        if (!html.includes(`id="${section}"`)) fail(`${file}: section #${section} is missing`)
      }
      checkPlan(file, html, ['d1', 'd2', 'd3', 'd4', 'd5', 'd6', 'd7'])
      if (!html.includes('Apache License 2.0') && lang === 'en')
        fail(`${file}: license mention missing`)
      if (!html.includes('application/ld+json')) fail(`${file}: no JSON-LD`)
    }
    if (id === 'registries') {
      if (count(html, /<app-group-sim/g) !== 1) fail(`${file}: group demo missing`)
      if (!/class="sim__job"[^>]*data-state="success"/.test(html))
        fail(`${file}: the group demo should be prerendered finished`)
      if (count(html, /class="spec__row"/g) !== 6) fail(`${file}: six repository rules expected`)
      if (!html.includes('docker push')) fail(`${file}: the Docker commands are missing`)
    }
    if (id === 'install') {
      if (count(html, /<app-install[ >]/g) !== 1) fail(`${file}: install tabs missing`)
      if (count(html, /class="install__panel[ "]/g) !== 3)
        fail(`${file}: three install panels expected`)
      if (count(html, /<gbt-copy-button/g) < 3) fail(`${file}: copy buttons missing`)
    }
    if (id === 'security' && count(html, /class="spec__row sec__row"/g) !== 10) {
      fail(`${file}: ten security facts expected`)
    }
    if (id === 'roadmap' && count(html, /<section[^>]*class="section group/g) !== 4) {
      fail(`${file}: expected 4 roadmap groups`)
    }
    // The example organization of the commands, the sibling site and the license of the structured data are text or links,
    // not resources.
    if (
      /https?:\/\/(?!www\.artiferris\.pro|app\.artiferris\.pro|acme\.artiferris\.pro|www\.ferrisgit\.pro|github\.com\/Masmarino|www\.apache\.org\/licenses|schema\.org|www\.w3\.org|www\.sitemaps\.org)[^"'\s<)]+/.test(
        html.replace(/<svg[\s\S]*?<\/svg>/g, ''),
      )
    ) {
      fail(`${file}: a third-party URL is referenced`)
    }
  }
}

const notFound = read('404.html')
if (notFound !== null) {
  if (!notFound.includes('<html lang="en"')) fail('404.html is not in English')
  for (const lang of site.langs) {
    if (!notFound.includes(`href="/${lang}/"`)) fail(`404.html does not link to /${lang}/`)
  }
}

const sitemap = read('sitemap.xml')
if (sitemap !== null) {
  const locs = count(sitemap, /<loc>/g)
  const expected = site.langs.length * Object.keys(site.pages).length
  if (locs !== expected) fail(`sitemap.xml has ${locs} URLs, expected ${expected}`)
}
const robots = read('robots.txt')
if (robots !== null && !robots.includes(`Sitemap: ${site.siteUrl}/sitemap.xml`)) {
  fail('robots.txt does not point at the sitemap')
}
// Screenshots, in both themes: crops of at most 140 kB, whole screens of at most 250 kB.
for (const name of ['repositories', 'package', 'scan', 'organizations', 'explore', 'docs']) {
  for (const theme of ['light', 'dark']) {
    for (const [suffix, limit] of [
      ['-detail', 140_000],
      ['', 250_000],
    ]) {
      const file = `images/screens/${name}${suffix}-${theme}.webp`
      read(file)
      const path = join(dist, file)
      if (existsSync(path) && statSync(path).size > limit) fail(`${file} is over ${limit} bytes`)
    }
  }
}

const FONTS = [
  'fonts/ibm-plex-sans-400.woff2',
  'fonts/ibm-plex-sans-500.woff2',
  'fonts/ibm-plex-sans-600.woff2',
  'fonts/ibm-plex-sans-condensed-600.woff2',
  'fonts/ibm-plex-mono-400.woff2',
  'fonts/ibm-plex-mono-500.woff2',
]
for (const file of [
  ...site.langs.map((lang) => `images/og-${lang}.png`),
  'favicon.ico',
  'theme-init.js',
  ...FONTS,
]) {
  read(file)
}
read('fonts/OFL-ibm-plex.txt')
// The self-hosted fonts stay under 130 kB together.
const fontBytes = FONTS.reduce((sum, file) => {
  const path = join(dist, file)
  return sum + (existsSync(path) ? statSync(path).size : 0)
}, 0)
if (fontBytes > 130_000) fail(`fonts weigh ${fontBytes} bytes, over the 130 kB budget`)
const stylesheet = readdirSync(dist).find((name) => /^styles-.*\.css$/.test(name))
const css = stylesheet ? readFileSync(join(dist, stylesheet), 'utf-8') : ''
for (const font of FONTS) {
  if (!css.includes(`/${font}`)) fail(`the stylesheet does not load ${font}`)
}
if (!/font-display:\s*swap/.test(css)) fail('fonts are not font-display: swap')
// Raw size of the initial JavaScript.
const initialBytes = readdirSync(dist)
  .filter((name) => /^main-.*\.js$/.test(name))
  .reduce((sum, name) => sum + statSync(join(dist, name)).size, 0)
if (initialBytes > 450_000) fail(`main bundle weighs ${initialBytes} bytes, over 450 kB`)

if (errors.length > 0) {
  console.error(errors.map((error) => `check-dist: ${error}`).join('\n'))
  process.exit(1)
}
console.log(`check-dist: ${pages} pages, 404.html, sitemap.xml and robots.txt are in place`)
