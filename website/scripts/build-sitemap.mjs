// Writes public/sitemap.xml from src/app/seo/site.json. Runs before every build and `ng serve`; the file is not committed.
import { writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { readSite, root } from './paths.mjs'
import { buildSitemap, pageUrls } from './sitemap.mjs'

const site = readSite()

writeFileSync(join(root, 'public/sitemap.xml'), buildSitemap(site))
console.log(`sitemap.xml: ${pageUrls(site).length} URLs`)
