import { t } from '../../shared/i18n/translator'
import { TranslocoPipe } from '@jsverse/transloco'
import { ChangeDetectionStrategy, Component, inject, signal } from '@angular/core'
import { Button, Card } from '@masmarino/gabarit'
import { AuthService } from '../../auth/application/auth.service'
import { SessionRevocationService } from '../../auth/application/session-revocation.service'
import { ConfirmService } from '../../shared/confirm.service'
import { ToastService } from '../../shared/toast.service'

@Component({
  selector: 'app-session-settings',
  standalone: true,
  imports: [TranslocoPipe, Button, Card],
  templateUrl: './session-settings.html',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class SessionSettings {
  private readonly auth = inject(AuthService)
  private readonly sessionRevocation = inject(SessionRevocationService)
  private readonly confirmService = inject(ConfirmService)
  private readonly toastService = inject(ToastService)

  readonly signingOut = signal(false)

  async logoutEverywhere(): Promise<void> {
    if (this.signingOut()) {
      return
    }
    const confirmed = await this.confirmService.ask({
      heading: t('account.sessions.signOutEverywhere'),
      message: t('account.sessions.confirmMessage'),
      confirmLabel: t('account.sessions.signOutEverywhere'),
      danger: true,
    })
    if (!confirmed) {
      return
    }
    this.signingOut.set(true)
    this.auth.logoutEverywhere().subscribe({
      next: () => {
        this.signingOut.set(false)
        this.sessionRevocation.signOutAndRedirect()
      },
      error: () => {
        this.signingOut.set(false)
        this.toastService.error(t('account.sessions.errors.signOutFailed'))
      },
    })
  }
}
