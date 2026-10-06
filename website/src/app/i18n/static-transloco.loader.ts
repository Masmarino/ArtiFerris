import { Injectable } from '@angular/core'
import { Translation, TranslocoLoader } from '@jsverse/transloco'
import { LANGS, isLang } from './languages'

// Dictionaries are bundled, not fetched, so the prerender needs no HTTP. Each one is its own chunk, so a visitor only
// downloads the language they read.
const LOADERS: Record<string, () => Promise<{ default: Translation }>> = {
  en: () => import('./en.json'),
  fr: () => import('./fr.json'),
  it: () => import('./it.json'),
  es: () => import('./es.json'),
  de: () => import('./de.json'),
}

export const AVAILABLE_LANGS = [...LANGS]

@Injectable({ providedIn: 'root' })
export class StaticTranslocoLoader implements TranslocoLoader {
  async getTranslation(lang: string): Promise<Translation> {
    if (!isLang(lang)) return {}
    return (await LOADERS[lang]()).default
  }
}
