import { ChangeDetectionStrategy, Component, OnInit, inject, signal } from '@angular/core'
import {
  FormControl,
  FormGroup,
  FormsModule,
  ReactiveFormsModule,
  Validators,
} from '@angular/forms'
import { ActivatedRoute, Router, RouterLink } from '@angular/router'
import { firstValueFrom } from 'rxjs'
import { Alert, Button, Divider, GbtInput } from '@masmarino/gabarit'
import { TranslocoPipe } from '@jsverse/transloco'
import { AuthService } from '../application/auth.service'
import { safeReturnUrl } from '../domain/return-url'
import {
  SESSIONS_ENDED_MESSAGE_KEY,
  SESSIONS_ENDED_QUERY_PARAM,
  SESSIONS_ENDED_REASON,
} from '../domain/sessions-ended'
import { getPasskeyAssertion, passkeysSupported } from '../../shared/webauthn-browser'
import { MfaEnrollmentPage } from '../mfa-enrollment/mfa-enrollment'
import { overloadMessage } from '../../shared/api-error'
import { t } from '../../shared/i18n/translator'

const LOGIN_OVERLOAD = {
  busy: 'auth.login.errors.busy',
  tooManyRequests: 'auth.login.errors.tooManyRequests',
}

@Component({
  selector: 'app-login-page',
  standalone: true,
  imports: [
    ReactiveFormsModule,
    FormsModule,
    GbtInput,
    Button,
    MfaEnrollmentPage,
    RouterLink,
    Alert,
    Divider,
    TranslocoPipe,
  ],
  templateUrl: './login-page.html',
  styleUrl: './login-page.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class LoginPage implements OnInit {
  private readonly auth = inject(AuthService)
  private readonly router = inject(Router)
  private readonly queryParams = inject(ActivatedRoute).snapshot.queryParamMap
  private readonly returnUrl = safeReturnUrl(this.queryParams.get('returnUrl'))
  readonly sessionsEndedMessage =
    this.queryParams.get(SESSIONS_ENDED_QUERY_PARAM) === SESSIONS_ENDED_REASON
      ? t(SESSIONS_ENDED_MESSAGE_KEY)
      : null

  readonly form = new FormGroup({
    username: new FormControl('', { nonNullable: true, validators: [Validators.required] }),
    password: new FormControl('', { nonNullable: true, validators: [Validators.required] }),
  })

  readonly errorMessage = signal<string | null>(null)
  readonly submitting = signal(false)
  readonly ssoType = signal<'ldap' | 'oidc' | null>(null)
  // Visible by default: a slow check must not hide the sign-up link.
  readonly registrationEnabled = signal(true)
  readonly ssoLinkInvalid = signal(false)

  ngOnInit(): void {
    const hashParams = new URLSearchParams(window.location.hash.replace(/^#/, ''))
    const token = hashParams.get('token')
    if (token !== null) {
      // Clear the fragment so the token does not stay in history.
      history.replaceState(null, '', window.location.pathname + window.location.search)
      const completed = token ? this.auth.completeExternalLogin(token) : null
      if (completed) {
        this.navigateAfterLogin(completed.returnUrl)
        return
      }
      this.ssoLinkInvalid.set(true)
    } else {
      this.auth.abandonSsoLogin()
    }

    this.auth.getSsoConfig().subscribe({
      next: (config) => {
        this.ssoType.set(config.type)
        this.registrationEnabled.set(config.registration_enabled)
      },
      // Local login stays available if this check fails.
      error: () => this.ssoType.set(null),
    })
  }

  startSso(): void {
    this.auth.beginSsoLogin(this.returnUrl)
  }

  readonly mfaToken = signal<string | null>(null)
  readonly mfaSetupRequired = signal(false)
  readonly mfaHasTotp = signal(false)
  readonly mfaHasPasskey = signal(false)
  readonly useBackupCode = signal(false)
  readonly mfaForm = new FormGroup({
    code: new FormControl('', { nonNullable: true, validators: [Validators.required] }),
  })
  readonly passkeysSupported = passkeysSupported()

  submit(): void {
    if (this.form.invalid || this.submitting()) {
      return
    }
    this.submitting.set(true)
    this.errorMessage.set(null)
    const { username, password } = this.form.getRawValue()
    const attempt =
      this.ssoType() === 'ldap'
        ? this.auth.loginWithLdap(username, password)
        : this.auth.login(username, password)
    attempt.subscribe({
      next: (outcome) => {
        if (outcome.mfaRequired && outcome.mfaToken) {
          this.submitting.set(false)
          this.mfaToken.set(outcome.mfaToken)
          this.mfaSetupRequired.set(!!outcome.mfaSetupRequired)
          this.mfaHasTotp.set(!!outcome.mfaHasTotp)
          this.mfaHasPasskey.set(!!outcome.mfaHasPasskey)
        } else {
          this.navigateAfterLogin()
        }
      },
      error: (error: unknown) => {
        this.submitting.set(false)
        this.errorMessage.set(
          overloadMessage(error, LOGIN_OVERLOAD) ?? t('auth.login.errors.invalidCredentials'),
        )
      },
    })
  }

  toggleBackupCode(): void {
    this.useBackupCode.update((value) => !value)
    this.mfaForm.reset()
    this.errorMessage.set(null)
  }

  submitMfa(): void {
    const mfaToken = this.mfaToken()
    if (!mfaToken || this.mfaForm.invalid || this.submitting()) {
      return
    }
    this.submitting.set(true)
    this.errorMessage.set(null)
    const { code } = this.mfaForm.getRawValue()
    const verify = this.useBackupCode()
      ? this.auth.verifyMfa(mfaToken, undefined, code)
      : this.auth.verifyMfa(mfaToken, code, undefined)
    verify.subscribe({
      next: () => this.navigateAfterLogin(),
      error: () => {
        this.submitting.set(false)
        this.errorMessage.set(
          t(
            this.useBackupCode()
              ? 'auth.login.errors.invalidBackupCode'
              : 'auth.login.errors.invalidCode',
          ),
        )
      },
    })
  }

  async submitPasskey(): Promise<void> {
    const mfaToken = this.mfaToken()
    if (!mfaToken || this.submitting()) {
      return
    }
    this.submitting.set(true)
    this.errorMessage.set(null)
    try {
      const start = await firstValueFrom(this.auth.startMfaPasskey(mfaToken))
      const credential = await getPasskeyAssertion(start.public_key)
      await firstValueFrom(this.auth.finishMfaPasskey(mfaToken, start.challenge_id, credential))
      this.navigateAfterLogin()
    } catch {
      this.submitting.set(false)
      this.errorMessage.set(t('auth.login.errors.passkeyFailed'))
    }
  }

  onEnrollmentCompleted(): void {
    this.navigateAfterLogin()
  }

  private navigateAfterLogin(returnUrl: string | null = this.returnUrl): void {
    void this.router.navigateByUrl(returnUrl ?? '/')
  }
}
