import { t } from '../../shared/i18n/translator'
import { TranslocoPipe } from '@jsverse/transloco'
import {
  ChangeDetectionStrategy,
  Component,
  computed,
  effect,
  inject,
  input,
  signal,
} from '@angular/core'
import { FormsModule } from '@angular/forms'
import { Button, Card, Checkbox, GbtInput, Spinner } from '@masmarino/gabarit'
import { SystemSettingsService } from '../application/system-settings.service'
import { ToastService } from '../../shared/toast.service'
import { MeService } from '../../shell/application/me.service'
import { PUBLIC_ORGANIZATION_ID } from '../domain/organization.entity'

interface FieldSpec {
  key: 'maxLoginAttempts' | 'loginAttemptWindowSeconds' | 'sessionTtlHours'
  min: number
  max: number
  labelKey: string
}

// Mirrors the backend's validation in UpdateSystemSettingsUseCase, kept in sync by hand, so the form can reject an out-of-range value before a round trip.
const FIELDS: FieldSpec[] = [
  { key: 'maxLoginAttempts', min: 1, max: 1000, labelKey: 'admin.system.maxLoginAttempts' },
  {
    key: 'loginAttemptWindowSeconds',
    min: 1,
    max: 86_400,
    labelKey: 'admin.system.loginAttemptWindow',
  },
  { key: 'sessionTtlHours', min: 1, max: 720, labelKey: 'admin.system.sessionTtl' },
]

@Component({
  selector: 'app-system-settings',
  standalone: true,
  imports: [TranslocoPipe, Button, Card, GbtInput, Checkbox, FormsModule, Spinner],
  templateUrl: './system-settings.html',
  styleUrl: './system-settings.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class SystemSettingsAdmin {
  private readonly settingsService = inject(SystemSettingsService)
  private readonly toastService = inject(ToastService)
  private readonly me = inject(MeService)

  /** Set only when embedded in an organization's own admin page — scopes read/write to it. */
  readonly organizationId = input<string | undefined>(undefined)

  readonly fields = FIELDS
  readonly maxLoginAttempts = signal('')
  readonly loginAttemptWindowSeconds = signal('')
  readonly sessionTtlHours = signal('')
  readonly registrationEnabled = signal(true)
  readonly seoIndexingEnabled = signal(false)
  readonly seoIndexingBlocked = signal(false)
  readonly publicPageEnabled = signal(true)
  readonly loading = signal(true)
  readonly loadFailed = signal(false)
  readonly saving = signal(false)

  // effect(), not ngOnInit — this component is reused across organizations on the same route.
  constructor() {
    effect(() => {
      const organizationId = this.organizationId()
      const stillCurrent = () => this.organizationId() === organizationId
      this.loading.set(true)
      this.loadFailed.set(false)
      this.settingsService.get(organizationId).subscribe({
        next: (settings) => {
          if (!stillCurrent()) {
            return
          }
          this.maxLoginAttempts.set(String(settings.max_login_attempts))
          this.loginAttemptWindowSeconds.set(String(settings.login_attempt_window_seconds))
          this.sessionTtlHours.set(String(settings.session_ttl_hours))
          this.registrationEnabled.set(settings.registration_enabled)
          this.seoIndexingEnabled.set(settings.seo_indexing_enabled)
          this.seoIndexingBlocked.set(settings.seo_indexing_blocked)
          this.publicPageEnabled.set(settings.public_page_enabled)
          this.loading.set(false)
        },
        error: () => {
          if (stillCurrent()) {
            this.loadFailed.set(true)
            this.loading.set(false)
          }
        },
      })
    })
  }

  // The public organization's row stands for the whole instance: its indexing switch and its public-page switch belong to a super-admin.
  private readonly isInstanceScope = computed(
    () => (this.organizationId() ?? this.me.organizationId()) === PUBLIC_ORGANIZATION_ID,
  )

  readonly showSeoIndexing = computed(() => this.me.isSuperAdmin() && this.isInstanceScope())

  /** Closing the public pages: an organization's own, or the whole instance's for a super-admin. */
  readonly showPublicPage = computed(() => !this.isInstanceScope() || this.me.isSuperAdmin())

  readonly publicPageForInstance = this.isInstanceScope

  /** Keeping search engines away from one organization; the instance has its indexing switch instead. */
  readonly showBlockIndexing = computed(() => !this.isInstanceScope())

  value(key: FieldSpec['key']): string {
    return this[key]()
  }

  setValue(key: FieldSpec['key'], value: string): void {
    this[key].set(value)
  }

  // Errors stay hidden until a save is attempted — same shape as smtp-settings.ts.
  readonly attemptedSave = signal(false)

  private rawFieldError(field: FieldSpec): string | null {
    const parsed = Number(this.value(field.key))
    if (this.value(field.key).trim() === '' || !Number.isInteger(parsed)) {
      return t('admin.system.errors.integer')
    }
    if (parsed < field.min || parsed > field.max) {
      return t('admin.system.errors.range', { min: field.min, max: field.max })
    }
    return null
  }

  fieldError(field: FieldSpec): string | null {
    return this.attemptedSave() ? this.rawFieldError(field) : null
  }

  readonly hasErrors = computed(
    () => this.attemptedSave() && this.fields.some((f) => this.rawFieldError(f) !== null),
  )

  save(): void {
    this.attemptedSave.set(true)
    if (this.hasErrors()) {
      return
    }
    this.saving.set(true)
    this.settingsService
      .update(
        {
          max_login_attempts: Number(this.maxLoginAttempts()),
          login_attempt_window_seconds: Number(this.loginAttemptWindowSeconds()),
          session_ttl_hours: Number(this.sessionTtlHours()),
          registration_enabled: this.registrationEnabled(),
          seo_indexing_enabled: this.seoIndexingEnabled(),
          seo_indexing_blocked: this.seoIndexingBlocked(),
          public_page_enabled: this.publicPageEnabled(),
        },
        this.organizationId(),
      )
      .subscribe({
        next: () => {
          this.saving.set(false)
          this.toastService.success(t('admin.system.saved'))
        },
        error: () => {
          this.saving.set(false)
          this.toastService.error(t('admin.system.errors.updateFailed'))
        },
      })
  }
}
