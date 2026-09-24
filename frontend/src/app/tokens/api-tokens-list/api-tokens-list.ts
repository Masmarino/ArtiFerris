import { ChangeDetectionStrategy, Component, OnInit, computed, inject, signal } from '@angular/core'
import { toSignal } from '@angular/core/rxjs-interop'
import { FormControl, ReactiveFormsModule, Validators } from '@angular/forms'
import { DatePipe } from '@angular/common'
import { HttpErrorResponse } from '@angular/common/http'
import { Button, EmptyState, GbtInput, Modal, Table, TableColumn } from '@masmarino/gabarit'
import { API_TOKEN_LIFETIME_DAYS, ApiToken } from '../domain/api-token.entity'
import { ApiTokensApplicationService } from '../application/api-tokens.application-service'
import { ConfirmService } from '../../shared/confirm.service'
import { badRequestMessage } from '../../shared/api-error'

const MAX_LABEL_LENGTH = 100
const WRONG_PASSWORD = 'Mot de passe incorrect.'

function createErrorMessage(error: unknown): string {
  const status = error instanceof HttpErrorResponse ? error.status : 0
  if (status === 429) {
    return 'Trop de tentatives, réessayez plus tard.'
  }
  if (status === 401 || status === 403) {
    return WRONG_PASSWORD
  }
  const message = badRequestMessage(error)
  if (message) {
    return message.toLowerCase().includes('invalid credentials') ? WRONG_PASSWORD : message
  }
  return 'Échec de la création du token.'
}

@Component({
  selector: 'app-api-tokens-list',
  standalone: true,
  imports: [ReactiveFormsModule, Table, Button, GbtInput, Modal, EmptyState],
  providers: [DatePipe],
  templateUrl: './api-tokens-list.html',
  styleUrl: './api-tokens-list.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ApiTokensList implements OnInit {
  private readonly tokenService = inject(ApiTokensApplicationService)
  private readonly datePipe = inject(DatePipe)
  private readonly confirmService = inject(ConfirmService)

  readonly tokens = signal<ApiToken[]>([])
  readonly showCreateForm = signal(false)
  readonly createdToken = signal<string | null>(null)
  readonly maxLabelLength = MAX_LABEL_LENGTH
  readonly newLabel = new FormControl('', {
    nonNullable: true,
    validators: [Validators.maxLength(MAX_LABEL_LENGTH)],
  })
  readonly newPassword = new FormControl('', { nonNullable: true })
  readonly createError = signal<string | null>(null)
  /** How long the token just created stays valid. */
  readonly createdTokenDays = signal<number>(API_TOKEN_LIFETIME_DAYS.session)

  private readonly typedPassword = toSignal(this.newPassword.valueChanges, { initialValue: '' })
  readonly lifetimeDays = computed(() =>
    this.typedPassword()
      ? API_TOKEN_LIFETIME_DAYS.reauthenticated
      : API_TOKEN_LIFETIME_DAYS.session,
  )

  readonly columns: TableColumn<ApiToken>[] = [
    { key: 'label', label: 'Nom' },
    {
      key: 'created_at',
      label: 'Créé le',
      format: (t) => this.datePipe.transform(t.created_at, 'short') ?? '',
    },
  ]
  readonly rowId = (t: ApiToken): string => t.id

  ngOnInit(): void {
    this.reload()
  }

  reload(): void {
    this.tokenService.list().subscribe((tokens) => this.tokens.set(tokens))
  }

  readonly creatingToken = signal(false)

  createToken(): void {
    const label = this.newLabel.value.trim()
    if (!label || this.creatingToken()) return
    const password = this.newPassword.value
    const days = this.lifetimeDays()
    this.creatingToken.set(true)
    this.createError.set(null)
    this.tokenService.create(label, password || null).subscribe({
      next: (result) => {
        this.creatingToken.set(false)
        this.createdToken.set(result.token)
        this.createdTokenDays.set(days)
        this.newLabel.setValue('')
        this.newPassword.setValue('')
        this.showCreateForm.set(false)
        this.reload()
      },
      error: (error: unknown) => {
        this.creatingToken.set(false)
        this.newPassword.setValue('')
        this.createError.set(createErrorMessage(error))
      },
    })
  }

  readonly revokeError = signal<string | null>(null)

  async revokeToken(token: ApiToken): Promise<void> {
    const confirmed = await this.confirmService.ask({
      heading: 'Révoquer le token',
      message: `Révoquer le token "${token.label}" ? Toute intégration npm qui l'utilise cessera de fonctionner.`,
      confirmLabel: 'Révoquer',
      danger: true,
    })
    if (!confirmed) return
    this.revokeError.set(null)
    this.tokenService.revoke(token.id).subscribe({
      next: () => this.reload(),
      error: () => this.revokeError.set(`Échec de la révocation du token "${token.label}".`),
    })
  }

  closeCreateForm(): void {
    this.showCreateForm.set(false)
    this.newPassword.setValue('')
    this.createError.set(null)
  }

  dismissCreatedToken(): void {
    this.createdToken.set(null)
  }
}
