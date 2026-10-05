import { activeLocale, t } from '../../shared/i18n/translator'
import { provideAuthKit } from '../../auth/kit/auth-kit'
import { TranslocoPipe } from '@jsverse/transloco'
import { LanguageSettings } from '../language-settings/language-settings'
import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core'
import { toSignal } from '@angular/core/rxjs-interop'
import {
  AbstractControl,
  FormControl,
  FormGroup,
  ReactiveFormsModule,
  ValidationErrors,
  Validators,
} from '@angular/forms'
import { HttpErrorResponse } from '@angular/common/http'
import { ActivatedRoute, Router, RouterLink } from '@angular/router'
import { map } from 'rxjs'
import { Avatar } from '@masmarino/gabarit/avatar'
import { Badge } from '@masmarino/gabarit/badge'
import { Button } from '@masmarino/gabarit/button'
import { Card } from '@masmarino/gabarit/card'
import { Icon } from '@masmarino/gabarit/icon'
import { GbtInput } from '@masmarino/gabarit/input'
import { MfaSettings } from '@masmarino/gabarit/mfa-settings'
import { NavTab, NavTabs } from '@masmarino/gabarit/nav-tabs'
import { PageLayout } from '@masmarino/gabarit/page-layout'
import { PasskeySettings } from '@masmarino/gabarit/passkey-settings'
import { SaveStatus } from '@masmarino/gabarit/save-status'
import { Skeleton } from '@masmarino/gabarit/skeleton'
import { ApiTokensList } from '../../tokens/api-tokens-list/api-tokens-list'
import { AuthService } from '../../auth/application/auth.service'
import { MeService } from '../../shell/application/me.service'
import { LocalizedDatePipe } from '../../shared/i18n/localized-date'
import { SessionSettings } from '../session-settings/session-settings'
import { PageHeading } from '../../shared/page-heading/page-heading'
import { ToastService } from '../../shared/toast.service'
import { SessionRevocationService } from '../../auth/application/session-revocation.service'

const MIN_PASSWORD_LENGTH = 8

type AccountSectionKey = 'profile' | 'password' | 'security' | 'tokens'

const SECTIONS: { key: AccountSectionKey; labelKey: string; icon: string }[] = [
  { key: 'profile', labelKey: 'account.sections.profile', icon: 'user' },
  { key: 'password', labelKey: 'account.sections.password', icon: 'lock' },
  { key: 'security', labelKey: 'account.sections.security', icon: 'shield-check' },
  { key: 'tokens', labelKey: 'account.sections.tokens', icon: 'key' },
]

const DEFAULT_SECTION: AccountSectionKey = 'profile'

function passwordsMatch(group: AbstractControl): ValidationErrors | null {
  const newPassword = group.get('newPassword')?.value
  const confirmPassword = group.get('confirmPassword')?.value
  return newPassword === confirmPassword ? null : { passwordsMismatch: true }
}

/**
 * The account's settings, laid out like FerrisGit's: a section menu on the side, one section at a
 * time. The section lives in the URL (`?section=<key>`, none for the profile) so it can be
 * deep-linked; an unknown key shows the profile.
 */
@Component({
  selector: 'app-account-page',
  standalone: true,
  imports: [
    TranslocoPipe,
    LanguageSettings,
    ReactiveFormsModule,
    RouterLink,
    Avatar,
    Badge,
    Button,
    Card,
    GbtInput,
    Icon,
    NavTab,
    NavTabs,
    PageHeading,
    PageLayout,
    SaveStatus,
    Skeleton,
    ApiTokensList,
    LocalizedDatePipe,
    MfaSettings,
    PasskeySettings,
    SessionSettings,
  ],
  // The security settings are Gabarit's, on our API and in the active language.
  providers: [provideAuthKit()],
  templateUrl: './account-page.html',
  styleUrl: './account-page.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class AccountPage {
  readonly me = inject(MeService)
  private readonly auth = inject(AuthService)
  private readonly router = inject(Router)
  private readonly route = inject(ActivatedRoute)
  private readonly toastService = inject(ToastService)
  private readonly sessionRevocation = inject(SessionRevocationService)

  protected readonly minPasswordLength = MIN_PASSWORD_LENGTH
  protected readonly locale = activeLocale

  protected readonly sections = SECTIONS.map((section) => ({
    ...section,
    queryParams: section.key === DEFAULT_SECTION ? undefined : { section: section.key },
  }))

  private readonly requestedSection = toSignal(
    this.route.queryParamMap.pipe(map((params) => params.get('section'))),
    { initialValue: null },
  )

  protected readonly activeSection = computed<AccountSectionKey>(() => {
    const requested = this.requestedSection()
    return SECTIONS.find((section) => section.key === requested)?.key ?? DEFAULT_SECTION
  })

  protected readonly avatarName = computed(() => (this.me.username() ?? '').replace(/[._-]+/g, ' '))

  readonly form = new FormGroup(
    {
      currentPassword: new FormControl('', {
        nonNullable: true,
        validators: [Validators.required],
      }),
      newPassword: new FormControl('', {
        nonNullable: true,
        validators: [Validators.required, Validators.minLength(MIN_PASSWORD_LENGTH)],
      }),
      confirmPassword: new FormControl('', {
        nonNullable: true,
        validators: [Validators.required],
      }),
    },
    { validators: passwordsMatch },
  )

  private readonly passwordValues = toSignal(this.form.valueChanges, {
    initialValue: this.form.getRawValue(),
  })

  protected readonly passwordLongEnough = computed(
    () => (this.passwordValues().newPassword ?? '').length >= MIN_PASSWORD_LENGTH,
  )
  /** The button only changes look (secondary until all three fields are filled), so it keeps focus. */
  protected readonly passwordReady = computed(() => {
    const { currentPassword, newPassword, confirmPassword } = this.passwordValues()
    return !!currentPassword && !!newPassword && !!confirmPassword
  })

  readonly submitting = signal(false)
  protected readonly passwordResult = signal<{ state: 'saved' | 'error'; message: string } | null>(
    null,
  )

  /** The server ended every session of the account (a factor or the backup codes changed): sign in again. */
  protected sessionsEnded(): void {
    this.sessionRevocation.signOutAndRedirect()
  }

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
    if (this.submitting()) return
    this.passwordResult.set(null)
    if (this.form.invalid) {
      // Shows what is missing or wrong, on the fields themselves.
      this.form.markAllAsTouched()
      return
    }
    this.submitting.set(true)
    const { currentPassword, newPassword } = this.form.getRawValue()
    this.me.changePassword(currentPassword, newPassword).subscribe({
      next: () => {
        // The other sessions ended; this one goes on with the fresh token MeService kept.
        this.submitting.set(false)
        this.form.reset({ currentPassword: '', newPassword: '', confirmPassword: '' })
        this.passwordResult.set({ state: 'saved', message: t('account.password.changed') })
        this.toastService.success(t('account.password.changed'))
      },
      error: (error: unknown) => {
        this.submitting.set(false)
        const status = error instanceof HttpErrorResponse ? error.status : 0
        // The length rule is checked before sending, so a 400 can only mean a wrong current password.
        const message =
          status === 400
            ? t('account.password.errors.wrongCurrent')
            : status === 429
              ? t('account.password.errors.tooMany')
              : t('account.password.errors.change')
        this.passwordResult.set({ state: 'error', message })
        this.toastService.error(t('account.password.errors.notChanged'))
        this.form.patchValue({ currentPassword: '' })
      },
    })
  }

  logout(): void {
    this.auth.logout()
    void this.router.navigateByUrl('/login')
  }
}
