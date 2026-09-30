import { t } from '../../shared/i18n/translator'
import { TranslocoPipe } from '@jsverse/transloco'
import { ChangeDetectionStrategy, Component, inject, signal } from '@angular/core'
import {
  AbstractControl,
  FormControl,
  FormGroup,
  ReactiveFormsModule,
  ValidationErrors,
  Validators,
} from '@angular/forms'
import { Button, Card, GbtInput, Tab, Tabs } from '@masmarino/gabarit'
import { ApiTokensList } from '../../tokens/api-tokens-list/api-tokens-list'
import { MeService } from '../../shell/application/me.service'
import { DatePipe } from '@angular/common'
import { MfaSettings } from '../mfa-settings/mfa-settings'
import { PasskeySettings } from '../passkey-settings/passkey-settings'
import { SessionSettings } from '../session-settings/session-settings'
import { ToastService } from '../../shared/toast.service'
import { SessionRevocationService } from '../../auth/application/session-revocation.service'

function passwordsMatch(group: AbstractControl): ValidationErrors | null {
  const newPassword = group.get('newPassword')?.value
  const confirmPassword = group.get('confirmPassword')?.value
  return newPassword === confirmPassword ? null : { passwordsMismatch: true }
}

@Component({
  selector: 'app-account-page',
  standalone: true,
  imports: [
    TranslocoPipe,
    ReactiveFormsModule,
    Button,
    GbtInput,
    ApiTokensList,
    Card,
    DatePipe,
    MfaSettings,
    PasskeySettings,
    SessionSettings,
    Tabs,
    Tab,
  ],
  templateUrl: './account-page.html',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class AccountPage {
  readonly me = inject(MeService)
  private readonly toastService = inject(ToastService)
  private readonly sessionRevocation = inject(SessionRevocationService)

  readonly form = new FormGroup(
    {
      currentPassword: new FormControl('', {
        nonNullable: true,
        validators: [Validators.required],
      }),
      newPassword: new FormControl('', {
        nonNullable: true,
        validators: [Validators.required, Validators.minLength(8)],
      }),
      confirmPassword: new FormControl('', {
        nonNullable: true,
        validators: [Validators.required],
      }),
    },
    { validators: passwordsMatch },
  )

  readonly submitting = signal(false)

  get confirmPasswordError(): string | null {
    const control = this.form.controls.confirmPassword
    return this.form.hasError('passwordsMismatch') && control.touched
      ? t('account.password.errors.mismatch')
      : null
  }

  get newPasswordError(): string | null {
    const control = this.form.controls.newPassword
    return control.hasError('minlength') && control.touched
      ? t('account.password.errors.tooShort')
      : null
  }

  submit(): void {
    if (this.form.invalid || this.submitting()) return
    this.submitting.set(true)
    const { currentPassword, newPassword } = this.form.getRawValue()
    this.me.changePassword(currentPassword, newPassword).subscribe({
      next: () => {
        this.submitting.set(false)
        this.form.reset({ currentPassword: '', newPassword: '', confirmPassword: '' })
        this.sessionRevocation.signOutAndRedirect()
      },
      error: () => {
        this.submitting.set(false)
        this.toastService.error(t('account.password.errors.change'))
        this.form.patchValue({ currentPassword: '' })
      },
    })
  }
}
