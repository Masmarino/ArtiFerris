import { DOCUMENT } from '@angular/common'
import { Injectable, inject } from '@angular/core'
import { TranslocoService } from '@jsverse/transloco'
import { firstValueFrom } from 'rxjs'
import { Language } from './languages'
import { activeLanguage, setActiveLanguage } from './translator'

@Injectable({ providedIn: 'root' })
export class LanguageService {
  private readonly transloco = inject(TranslocoService)
  private readonly document = inject(DOCUMENT)

  readonly language = activeLanguage

  async use(language: Language): Promise<void> {
    await firstValueFrom(this.transloco.load(language), { defaultValue: undefined })
    this.transloco.setActiveLang(language)
    this.document.documentElement.lang = language
    setActiveLanguage(language)
  }
}
