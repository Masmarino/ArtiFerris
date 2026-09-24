import { ChangeDetectionStrategy, Component, OnInit, inject, signal } from '@angular/core'
import { FormsModule } from '@angular/forms'
import { HttpErrorResponse } from '@angular/common/http'
import * as QRCode from 'qrcode'
import { Button, Card, Divider, GbtInput, Spinner, Tooltip } from '@masmarino/gabarit'
import { MfaService } from '../application/mfa.service'
import { MfaStatus } from '../domain/mfa.types'
import { ToastService } from '../../shared/toast.service'
import { SessionRevocationService } from '../../auth/application/session-revocation.service'

type ViewState = 'loading' | 'load-failed' | 'disabled' | 'enrolling' | 'backup-codes' | 'enabled'

@Component({
  selector: 'app-mfa-settings',
  standalone: true,
  imports: [Button, Card, GbtInput, FormsModule, Spinner, Tooltip, Divider],
  templateUrl: './mfa-settings.html',
  styleUrl: './mfa-settings.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class MfaSettings implements OnInit {
  private readonly mfaService = inject(MfaService)
  private readonly toastService = inject(ToastService)
  private readonly sessionRevocation = inject(SessionRevocationService)

  readonly state = signal<ViewState>('loading')
  readonly status = signal<MfaStatus | null>(null)
  readonly errorMessage = signal<string | null>(null)
  readonly submitting = signal(false)

  readonly enrollmentSecret = signal('')
  readonly qrCodeDataUrl = signal<string | null>(null)
  readonly confirmCode = signal('')

  readonly backupCodes = signal<string[]>([])

  readonly disablePassword = signal('')
  readonly regeneratePassword = signal('')
  readonly regeneratedCodes = signal<string[] | null>(null)

  ngOnInit(): void {
    this.reload()
  }

  retryLoad(): void {
    this.state.set('loading')
    this.reload()
  }

  private reload(): void {
    this.mfaService.getStatus().subscribe({
      next: (status) => {
        this.status.set(status)
        this.state.set(status.totp_enabled ? 'enabled' : 'disabled')
      },
      error: () => this.state.set('load-failed'),
    })
  }

  startEnrollment(): void {
    this.errorMessage.set(null)
    this.mfaService.enrollTotp().subscribe({
      next: (enrollment) => {
        this.enrollmentSecret.set(enrollment.secret)
        QRCode.toDataURL(enrollment.otpauth_url)
          .then((dataUrl) => this.qrCodeDataUrl.set(dataUrl))
          .catch(() => this.qrCodeDataUrl.set(null))
        this.state.set('enrolling')
      },
      error: () => this.errorMessage.set("Échec du démarrage de l'activation. Réessayez."),
    })
  }

  cancelEnrollment(): void {
    this.confirmCode.set('')
    this.qrCodeDataUrl.set(null)
    this.state.set('disabled')
  }

  confirmEnrollment(): void {
    if (this.confirmCode().trim() === '' || this.submitting()) {
      return
    }
    this.submitting.set(true)
    this.errorMessage.set(null)
    this.mfaService.confirmTotp(this.confirmCode()).subscribe({
      next: (result) => {
        this.submitting.set(false)
        this.backupCodes.set(result.backup_codes)
        this.confirmCode.set('')
        this.state.set('backup-codes')
      },
      error: () => {
        this.submitting.set(false)
        this.errorMessage.set('Code invalide.')
      },
    })
  }

  acknowledgeBackupCodes(): void {
    this.backupCodes.set([])
    this.reload()
  }

  disable(): void {
    if (this.disablePassword().trim() === '' || this.submitting()) {
      return
    }
    this.submitting.set(true)
    this.mfaService.disableTotp(this.disablePassword()).subscribe({
      next: () => {
        this.submitting.set(false)
        this.disablePassword.set('')
        this.sessionRevocation.signOutAndRedirect()
      },
      error: (err: HttpErrorResponse) => {
        this.submitting.set(false)
        this.toastService.error(
          err.status === 400 ? 'Mot de passe incorrect.' : 'Échec de la désactivation.',
        )
      },
    })
  }

  regenerateBackupCodes(): void {
    if (this.regeneratePassword().trim() === '' || this.submitting()) {
      return
    }
    this.submitting.set(true)
    this.mfaService.regenerateBackupCodes(this.regeneratePassword()).subscribe({
      next: (result) => {
        this.submitting.set(false)
        this.regeneratePassword.set('')
        this.regeneratedCodes.set(result.backup_codes)
      },
      error: (err: HttpErrorResponse) => {
        this.submitting.set(false)
        this.toastService.error(
          err.status === 400 ? 'Mot de passe incorrect.' : 'Échec de la régénération.',
        )
      },
    })
  }

  /** The backend already revoked this session: the codes had to be shown before signing out. */
  dismissRegeneratedCodes(): void {
    this.regeneratedCodes.set(null)
    this.sessionRevocation.signOutAndRedirect()
  }
}
