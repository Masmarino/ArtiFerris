import { DOCUMENT } from '@angular/common'
import { Injectable, inject } from '@angular/core'
import { TranslocoService } from '@jsverse/transloco'
import { firstValueFrom } from 'rxjs'
import { Language } from './languages'
import { activeLanguage, setActiveLanguage } from './translator'

/** Switches the language the interface is displayed in. */
@Injectable({ providedIn: 'root' })
export class LanguageService {
  private readonly transloco = inject(TranslocoService)
  private readonly document = inject(DOCUMENT)

  readonly language = activeLanguage

  /**
   * Loads the dictionary first, then switches: the interface never shows keys, and what reads
   * `t()` or `language` changes once, with the new text already available.
   */
  async use(language: Language): Promise<void> {
    await firstValueFrom(this.transloco.load(language), { defaultValue: undefined })
    this.transloco.setActiveLang(language)
    this.document.documentElement.lang = language
    setActiveLanguage(language)
  }
}
