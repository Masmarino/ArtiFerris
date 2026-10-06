import { inject } from '@angular/core'
import { CanMatchFn, ResolveFn, UrlSegment } from '@angular/router'
import { TranslocoService } from '@jsverse/transloco'
import { firstValueFrom } from 'rxjs'
import { DEFAULT_LANG, Lang, isLang } from './languages'

/** An unknown language in the first segment is not a match, so the router falls through to the 404 route. */
export const langMatch: CanMatchFn = (_route, segments: UrlSegment[]) => isLang(segments[0]?.path)

async function activate(lang: Lang): Promise<Lang> {
  const transloco = inject(TranslocoService)
  // Load the dictionary here, before the page exists, so the first render (prerender and browser alike) already has the
  // right text.
  await firstValueFrom(transloco.load(lang))
  transloco.setActiveLang(lang)
  return lang
}

export const langResolver: ResolveFn<Lang> = (route) => {
  const lang = route.paramMap.get('lang')
  return activate(isLang(lang) ? lang : DEFAULT_LANG)
}

export const defaultLangResolver: ResolveFn<Lang> = () => activate(DEFAULT_LANG)
