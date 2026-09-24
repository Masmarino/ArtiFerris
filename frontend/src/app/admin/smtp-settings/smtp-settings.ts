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
import {
  Alert,
  Button,
  Card,
  GbtInput,
  Select,
  SelectOption,
  Spinner,
  Tooltip,
} from '@masmarino/gabarit'
import { SmtpSettingsService } from '../application/smtp-settings.service'
import { SmtpSecurity, isUnreadableSmtpSettings } from '../domain/smtp-settings.entity'
import { ToastService } from '../../shared/toast.service'
import {
  SECRET_UNREADABLE_MESSAGE,
  isSecretUnreadable,
  overloadMessage,
  secretFormFailureMessage,
} from '../../shared/api-error'

const SECURITY_OPTIONS: SelectOption<SmtpSecurity>[] = [
  { value: 'start_tls', label: 'STARTTLS (port 587 usuellement)' },
  { value: 'tls', label: 'TLS implicite (port 465 usuellement)' },
  { value: 'none', label: 'Aucun (réseau interne de confiance uniquement)' },
]

@Component({
  selector: 'app-smtp-settings',
  standalone: true,
  imports: [Alert, Button, Card, GbtInput, Select, FormsModule, Spinner, Tooltip],
  templateUrl: './smtp-settings.html',
  styleUrl: './smtp-settings.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class SmtpSettingsAdmin {
  private readonly settingsService = inject(SmtpSettingsService)
  private readonly toastService = inject(ToastService)

  /** Set only when embedded in an organization's own admin page — scopes read/write to it. */
  readonly organizationId = input<string | undefined>(undefined)

  readonly securityOptions = SECURITY_OPTIONS

  readonly host = signal('')
  readonly port = signal('587')
  readonly username = signal('')
  readonly password = signal('')
  readonly fromName = signal('ArtiFerris')
  readonly fromAddress = signal('')
  readonly security = signal<SmtpSecurity>('start_tls')
  readonly passwordSet = signal(false)
  /** The stored password cannot be decrypted by this server: it has to be typed again. */
  readonly secretUnreadable = signal(false)

  readonly loading = signal(true)
  readonly loadFailed = signal(false)
  readonly saving = signal(false)

  readonly testRecipient = signal('')
  readonly sendingTest = signal(false)

  // effect(), not ngOnInit — this component is reused across organizations on the same route.
  constructor() {
    effect(() => {
      const organizationId = this.organizationId()
      const stillCurrent = () => this.organizationId() === organizationId
      // Nothing typed for the previous organization may end up saved on this one.
      this.password.set('')
      this.attemptedSave.set(false)
      this.testRecipient.set('')
      this.saving.set(false)
      this.sendingTest.set(false)
      this.loading.set(true)
      this.loadFailed.set(false)
      this.settingsService.get(organizationId).subscribe({
        next: (settings) => {
          if (!stillCurrent()) {
            return
          }
          this.secretUnreadable.set(false)
          if (settings && !isUnreadableSmtpSettings(settings)) {
            this.host.set(settings.host)
            this.port.set(String(settings.port))
            this.username.set(settings.username)
            this.fromName.set(settings.from_name)
            this.fromAddress.set(settings.from_address)
            this.security.set(settings.security)
            this.passwordSet.set(settings.password_set)
          } else {
            this.secretUnreadable.set(settings !== null)
            // Reset, or a previous org's SMTP config lingers on screen for one with none.
            this.host.set('')
            this.port.set('587')
            this.username.set('')
            this.fromName.set('ArtiFerris')
            this.fromAddress.set('')
            this.security.set('start_tls')
            this.passwordSet.set(false)
          }
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

  // Errors stay hidden until a save is attempted — a freshly opened form isn't a mistake yet.
  readonly attemptedSave = signal(false)

  private readonly rawPortError = computed(() => {
    const parsed = Number(this.port())
    if (this.port().trim() === '' || !Number.isInteger(parsed)) {
      return 'Doit être un nombre entier.'
    }
    if (parsed < 1 || parsed > 65_535) {
      return 'Doit être entre 1 et 65535.'
    }
    return null
  })

  private readonly rawHostError = computed(() =>
    this.host().trim() === '' ? "L'hôte est requis." : null,
  )

  private readonly rawUsernameError = computed(() =>
    this.username().trim() === '' ? "L'identifiant est requis." : null,
  )

  private readonly rawFromAddressError = computed(() =>
    this.fromAddress().includes('@') ? null : 'Doit être une adresse e-mail valide.',
  )

  private readonly rawFromNameError = computed(() =>
    this.fromName().trim() === '' ? "Le nom d'expéditeur est requis." : null,
  )

  private readonly rawPasswordError = computed(() => {
    if (!this.passwordSet() && this.password().trim() === '') {
      return 'Le mot de passe est requis lors de la première configuration.'
    }
    return null
  })

  readonly hostError = computed(() => (this.attemptedSave() ? this.rawHostError() : null))
  readonly portError = computed(() => (this.attemptedSave() ? this.rawPortError() : null))
  readonly usernameError = computed(() => (this.attemptedSave() ? this.rawUsernameError() : null))
  readonly fromAddressError = computed(() =>
    this.attemptedSave() ? this.rawFromAddressError() : null,
  )
  readonly fromNameError = computed(() => (this.attemptedSave() ? this.rawFromNameError() : null))
  readonly passwordError = computed(() => (this.attemptedSave() ? this.rawPasswordError() : null))

  readonly hasErrors = computed(
    () =>
      this.attemptedSave() &&
      (this.rawHostError() !== null ||
        this.rawPortError() !== null ||
        this.rawUsernameError() !== null ||
        this.rawFromNameError() !== null ||
        this.rawFromAddressError() !== null ||
        this.rawPasswordError() !== null),
  )

  save(): void {
    this.attemptedSave.set(true)
    if (this.hasErrors()) {
      return
    }
    const organizationId = this.organizationId()
    const stillCurrent = () => this.organizationId() === organizationId
    this.saving.set(true)
    this.settingsService
      .update(
        {
          host: this.host(),
          port: Number(this.port()),
          username: this.username(),
          password: this.password().trim() === '' ? undefined : this.password(),
          from_name: this.fromName(),
          from_address: this.fromAddress(),
          security: this.security(),
        },
        this.organizationId(),
      )
      .subscribe({
        next: () => {
          if (stillCurrent()) {
            this.saving.set(false)
            this.passwordSet.set(true)
            this.secretUnreadable.set(false)
            this.password.set('')
          }
          this.toastService.success('Paramètres SMTP enregistrés.')
        },
        error: (error: unknown) => {
          if (stillCurrent()) {
            this.saving.set(false)
            if (isSecretUnreadable(error)) {
              this.secretUnreadable.set(true)
            }
          }
          this.toastService.error(
            secretFormFailureMessage(error, 'Échec de la mise à jour des paramètres SMTP.'),
          )
        },
      })
  }

  sendTest(): void {
    if (this.testRecipient().trim() === '') {
      return
    }
    const organizationId = this.organizationId()
    const stillCurrent = () => this.organizationId() === organizationId
    this.sendingTest.set(true)
    this.settingsService.sendTestEmail(this.testRecipient(), organizationId).subscribe({
      next: () => {
        if (stillCurrent()) {
          this.sendingTest.set(false)
        }
        this.toastService.success('E-mail de test envoyé.')
      },
      error: (error: unknown) => {
        if (stillCurrent()) {
          this.sendingTest.set(false)
          if (isSecretUnreadable(error)) {
            this.secretUnreadable.set(true)
          }
        }
        this.toastService.error(
          isSecretUnreadable(error)
            ? SECRET_UNREADABLE_MESSAGE
            : (overloadMessage(error) ??
                "Échec de l'envoi de l'e-mail de test. Vérifiez la configuration."),
        )
      },
    })
  }
}
