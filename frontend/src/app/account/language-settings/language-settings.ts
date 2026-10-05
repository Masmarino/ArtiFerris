import { ChangeDetectionStrategy, Component, inject, signal } from '@angular/core'
import { FormsModule } from '@angular/forms'
import { firstValueFrom } from 'rxjs'
import { SaveStatus, type SaveStatusState } from '@masmarino/gabarit/save-status'
import { Select, SelectOption } from '@masmarino/gabarit/select'
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

/** The interface's language, a field of the profile card: saved to the account as soon as it is chosen. */
@Component({
  selector: 'app-language-settings',
  standalone: true,
  imports: [TranslocoPipe, Select, SaveStatus, FormsModule],
  templateUrl: './language-settings.html',
  styleUrl: './language-settings.scss',
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
  readonly state = signal<SaveStatusState>('idle')

  async choose(value: string): Promise<void> {
    if (!isSupported(value) || value === this.current() || this.saving()) {
      return
    }
    const previous: Language = this.current()
    this.saving.set(true)
    this.state.set('saving')
    await this.languageService.use(value)
    try {
      await firstValueFrom(this.me.setLanguage(value))
      this.state.set('saved')
      this.toastService.success(t('account.language.saved'))
    } catch {
      // Not saved: keep the account's language.
      await this.languageService.use(previous)
      this.state.set('error')
      this.toastService.error(t('account.language.errors.saveFailed'))
    } finally {
      this.saving.set(false)
    }
  }
}
