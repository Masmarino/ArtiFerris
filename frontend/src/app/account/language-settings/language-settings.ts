import { ChangeDetectionStrategy, Component, inject, signal } from '@angular/core'
import { FormsModule } from '@angular/forms'
import { firstValueFrom } from 'rxjs'
import { Card, Select, SelectOption } from '@masmarino/gabarit'
import { TranslocoPipe } from '@jsverse/transloco'
import { MeService } from '../../shell/application/me.service'
import { LanguageService } from '../../shared/i18n/language.service'
import {
  LANGUAGE_NAMES,
  Language,
  SUPPORTED_LANGUAGES,
  isSupported,
} from '../../shared/i18n/languages'
import { t } from '../../shared/i18n/translator'
import { ToastService } from '../../shared/toast.service'

@Component({
  selector: 'app-language-settings',
  standalone: true,
  imports: [TranslocoPipe, Card, Select, FormsModule],
  templateUrl: './language-settings.html',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class LanguageSettings {
  private readonly languageService = inject(LanguageService)
  private readonly me = inject(MeService)
  private readonly toastService = inject(ToastService)

  readonly options: SelectOption<string>[] = SUPPORTED_LANGUAGES.map((language) => ({
    value: language,
    label: LANGUAGE_NAMES[language],
  }))
  readonly current = this.languageService.language
  readonly saving = signal(false)

  async choose(value: string): Promise<void> {
    if (!isSupported(value) || value === this.current() || this.saving()) {
      return
    }
    const previous: Language = this.current()
    this.saving.set(true)
    await this.languageService.use(value)
    try {
      await firstValueFrom(this.me.setLanguage(value))
      this.toastService.success(t('account.language.saved'))
    } catch {
      // Not saved: keep the account's language.
      await this.languageService.use(previous)
      this.toastService.error(t('account.language.errors.saveFailed'))
    } finally {
      this.saving.set(false)
    }
  }
}
