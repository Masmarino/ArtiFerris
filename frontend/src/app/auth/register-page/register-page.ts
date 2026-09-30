import { t } from '../../shared/i18n/translator'
import { TranslocoPipe } from '@jsverse/transloco'
import { ChangeDetectionStrategy, Component, inject, signal } from '@angular/core'
import { FormControl, FormGroup, ReactiveFormsModule, Validators } from '@angular/forms'
import { Router, RouterLink } from '@angular/router'
import { Button, Divider, GbtInput } from '@masmarino/gabarit'
import { AuthService } from '../application/auth.service'
import { MfaEnrollmentPage } from '../mfa-enrollment/mfa-enrollment'
import { errorCode, overloadMessage } from '../../shared/api-error'

@Component({
  selector: 'app-register-page',
  standalone: true,
  imports: [
    TranslocoPipe,
    ReactiveFormsModule,
    GbtInput,
    Button,
    MfaEnrollmentPage,
    RouterLink,
    Divider,
  ],
  templateUrl: './register-page.html',
  styleUrl: '../login-page/login-page.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class RegisterPage {
  private readonly auth = inject(AuthService)
  private readonly router = inject(Router)

  readonly form = new FormGroup({
    username: new FormControl('', {
      nonNullable: true,
      validators: [Validators.required, Validators.pattern(/^[A-Za-z][A-Za-z0-9_-]{2,31}$/)],
    }),
    email: new FormControl('', {
      nonNullable: true,
      validators: [Validators.required, Validators.email],
    }),
    password: new FormControl('', {
      nonNullable: true,
      validators: [Validators.required, Validators.minLength(8)],
    }),
  })

  readonly errorMessage = signal<string | null>(null)
  readonly submitting = signal(false)

  // null until registration returns an mfa_token — the template then swaps to enrollment.
  readonly mfaToken = signal<string | null>(null)

  submit(): void {
    if (this.form.invalid || this.submitting()) {
      return
    }
    this.submitting.set(true)
    this.errorMessage.set(null)
    const { username, email, password } = this.form.getRawValue()
    this.auth.register(username, email, password).subscribe({
      next: (outcome) => {
        this.submitting.set(false)
        if (outcome.mfaToken) {
          this.mfaToken.set(outcome.mfaToken)
        }
      },
      error: (err: unknown) => {
        this.submitting.set(false)
        this.errorMessage.set(overloadMessage(err) ?? this.messageFor(errorCode(err)))
      },
    })
  }

  onEnrollmentCompleted(): void {
    this.router.navigateByUrl('/')
  }

  private messageFor(code: string | null): string {
    switch (code) {
      case 'username_taken':
        return t('auth.register.errors.usernameTaken')
      case 'invalid_email':
        return t('auth.register.errors.invalidEmail')
      case 'password_too_short':
        return t('auth.register.errors.passwordTooShort')
      case 'registration_unavailable':
        return t('auth.register.errors.notAvailable')
      case 'registration_disabled':
        return t('auth.register.errors.disabled')
      case 'invalid_username':
        return t('auth.register.errors.invalidUsername')
      default:
        return t('auth.register.errors.generic')
    }
  }
}
