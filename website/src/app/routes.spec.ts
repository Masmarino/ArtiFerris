import { UrlSegment } from '@angular/router'
import { serverRoutes } from './app.routes.server'
import { routes } from './app.routes'
import { langMatch } from './i18n/lang-routing'
import { LANGS } from './i18n/languages'
import { TrailingSlashUrlSerializer } from './shared/trailing-slash-url-serializer'

describe('routes', () => {
  it('prerender one file per language and page, 30 in all, plus the 404 page', async () => {
    const urls: string[] = []
    for (const route of serverRoutes) {
      const { getPrerenderParams } = route as {
        getPrerenderParams?: () => Promise<Record<string, string>[]>
      }
      const params = (await getPrerenderParams?.()) ?? [{}]
      for (const param of params) {
        urls.push(route.path.replace(':lang', param['lang'] ?? ''))
      }
    }
    const pages = urls.filter((url) => url !== '404' && url !== '**')
    expect(pages).toHaveLength(30)
    for (const lang of LANGS) {
      expect(pages).toContain(lang)
      for (const page of ['features', 'registries', 'install', 'security', 'roadmap']) {
        expect(pages).toContain(`${lang}/${page}`)
      }
    }
    expect(urls).toContain('404')
  })

  it('matches only the five languages in the first segment', () => {
    const run = (path: string) =>
      (langMatch as (route: unknown, segments: UrlSegment[]) => boolean)({}, [
        new UrlSegment(path, {}),
      ])
    for (const lang of LANGS) expect(run(lang)).toBe(true)
    expect(run('xx')).toBe(false)
    expect(run('features')).toBe(false)
  })

  it('ends with a catch-all that shows the 404 page', () => {
    expect(routes[routes.length - 1].path).toBe('**')
  })
})

describe('TrailingSlashUrlSerializer', () => {
  const serializer = new TrailingSlashUrlSerializer()
  const serialize = (url: string) => serializer.serialize(serializer.parse(url))

  it('adds the trailing slash that nginx serves', () => {
    expect(serialize('/fr/features')).toBe('/fr/features/')
    expect(serialize('/fr')).toBe('/fr/')
    expect(serialize('/')).toBe('/')
    expect(serialize('/404.html')).toBe('/404.html')
  })

  it('keeps the query and the fragment after the slash', () => {
    expect(serialize('/en/roadmap?x=1#top')).toBe('/en/roadmap/?x=1#top')
  })
})
