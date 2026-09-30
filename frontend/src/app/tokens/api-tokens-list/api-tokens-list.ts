import { t } from '../../shared/i18n/translator'
import { TranslocoPipe } from '@jsverse/transloco'
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

function createErrorMessage(error: unknown): string {
  const status = error instanceof HttpErrorResponse ? error.status : 0
  if (status === 429) {
    return t('tokens.errors.tooMany')
  }
  if (status === 401 || status === 403) {
    return t('tokens.errors.wrongPassword')
  }
  const message = badRequestMessage(error)
  if (message) {
    return message.toLowerCase().includes('invalid credentials')
      ? t('tokens.errors.wrongPassword')
      : message
  }
  return t('tokens.errors.createFailed')
}

@Component({
  selector: 'app-api-tokens-list',
  standalone: true,
  imports: [TranslocoPipe, ReactiveFormsModule, Table, Button, GbtInput, Modal, EmptyState],
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
    { key: 'label', label: t('tokens.table.name') },
    {
      key: 'created_at',
      label: t('tokens.table.createdAt'),
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
      heading: t('tokens.revoke.heading'),
      message: t('tokens.revoke.message', { label: token.label }),
      confirmLabel: t('tokens.revoke.confirm'),
      danger: true,
    })
    if (!confirmed) return
    this.revokeError.set(null)
    this.tokenService.revoke(token.id).subscribe({
      next: () => this.reload(),
      error: () => this.revokeError.set(t('tokens.errors.revokeFailed', { label: token.label })),
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
