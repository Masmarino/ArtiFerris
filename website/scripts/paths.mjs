// Locations and configuration shared by the build scripts.
import { readFileSync } from 'node:fs'
import { join } from 'node:path'

export const root = join(import.meta.dirname, '..')
export const dist = join(root, 'dist/artiferris-website/browser')

/** The languages and pages that the router, the sitemap and check-dist all derive from. */
export const readSite = () => JSON.parse(readFileSync(join(root, 'src/app/seo/site.json'), 'utf-8'))
