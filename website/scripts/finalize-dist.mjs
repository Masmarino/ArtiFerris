// Angular prerenders the /404 route as 404/index.html. nginx wants a plain file for `error_page`, and the directory
// would answer 200 at /404/, so the file becomes 404.html and the directory goes.
import { existsSync, renameSync, rmSync } from 'node:fs'
import { join } from 'node:path'
import { dist } from './paths.mjs'

const prerendered = join(dist, '404/index.html')

if (!existsSync(prerendered)) {
  console.error('finalize-dist: 404/index.html was not prerendered')
  process.exit(1)
}
renameSync(prerendered, join(dist, '404.html'))
rmSync(join(dist, '404'), { recursive: true })
console.log('finalize-dist: 404.html written')
