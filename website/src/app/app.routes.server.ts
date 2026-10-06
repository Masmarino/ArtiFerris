import { RenderMode, ServerRoute } from '@angular/ssr'
import { LANGS, PAGES } from './i18n/languages'

const langParams = async () => LANGS.map((lang) => ({ lang }))

// One prerendered file per language and page: /en/, /en/features/, /en/ci/, /en/install/, /en/security/, /en/roadmap/, /fr/, ...
const pagePaths = Object.values(PAGES).map((path) => (path ? `:lang/${path}` : ':lang'))

export const serverRoutes: ServerRoute[] = [
  ...pagePaths.map((path) => ({
    path,
    renderMode: RenderMode.Prerender,
    getPrerenderParams: langParams,
  })),
  // finalize-dist.mjs turns the prerendered /404 into 404.html, which Angular doesn't emit itself.
  { path: '404', renderMode: RenderMode.Prerender },
  { path: '**', renderMode: RenderMode.Client },
]
