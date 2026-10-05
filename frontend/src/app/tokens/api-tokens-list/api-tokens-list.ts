import { activeLocale, t } from '../../shared/i18n/translator'
import { TranslocoPipe } from '@jsverse/transloco'
import {
  ChangeDetectionStrategy,
  Component,
  ElementRef,
  Injector,
  OnInit,
  afterNextRender,
  computed,
  inject,
  signal,
  viewChild,
} from '@angular/core'
import { toSignal } from '@angular/core/rxjs-interop'
import { FormControl, ReactiveFormsModule, Validators } from '@angular/forms'
import { HttpErrorResponse } from '@angular/common/http'
import { Alert } from '@masmarino/gabarit/alert'
import { Badge } from '@masmarino/gabarit/badge'
import { Button } from '@masmarino/gabarit/button'
import { Card } from '@masmarino/gabarit/card'
import { CopyField } from '@masmarino/gabarit/copy-field'
import { EmptyState } from '@masmarino/gabarit/empty-state'
import { formatRelativeTime } from '@masmarino/gabarit/format'
import { Icon } from '@masmarino/gabarit/icon'
import { GbtInput } from '@masmarino/gabarit/input'
import { ListRow } from '@masmarino/gabarit/list-row'
import { SkeletonList } from '@masmarino/gabarit/skeleton-list'
import { API_TOKEN_LIFETIME_DAYS, ApiToken } from '../domain/api-token.entity'
import { ApiTokensApplicationService } from '../application/api-tokens.application-service'
import { ConfirmService } from '../../shared/confirm.service'
import { ToastService } from '../../shared/toast.service'
import { LocalizedDatePipe, formatLocalizedDate } from '../../shared/i18n/localized-date'
import { badRequestMessage, errorCode } from '../../shared/api-error'

const MAX_LABEL_LENGTH = 100

/** Relative up to a month ("il y a 3 j"), then the date. */
const RELATIVE_OPTIONS = { style: 'short', maxUnit: 'day', absoluteAfterDays: 30 } as const

interface CreatedToken {
  id: string
  label: string
  token: string
  days: number
}

function createErrorMessage(error: unknown): string {
  const status = error instanceof HttpErrorResponse ? error.status : 0
  if (status === 429) {
    return t('tokens.errors.tooMany')
  }
  if (status === 401 || status === 403) {
    return t('tokens.errors.wrongPassword')
  }
  if (errorCode(error) === 'invalid_credentials') {
    return t('tokens.errors.wrongPassword')
  }
  const message = badRequestMessage(error)
  if (message) {
    return message
  }
  return t('tokens.errors.createFailed')
}

/**
 * The account's API tokens, laid out like FerrisGit's: a card to generate one, the new token shown
 * once in a card of its own, then the active tokens as rows.
 */
@Component({
  selector: 'app-api-tokens-list',
  standalone: true,
  imports: [
    TranslocoPipe,
    ReactiveFormsModule,
    Alert,
    Badge,
    Button,
    Card,
    CopyField,
    EmptyState,
    GbtInput,
    Icon,
    ListRow,
    LocalizedDatePipe,
    SkeletonList,
  ],
  templateUrl: './api-tokens-list.html',
  styleUrl: './api-tokens-list.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ApiTokensList implements OnInit {
  private readonly tokenService = inject(ApiTokensApplicationService)
  private readonly confirmService = inject(ConfirmService)
  private readonly toastService = inject(ToastService)
  private readonly injector = inject(Injector)

  private readonly revealCard = viewChild('revealCard', { read: ElementRef })
  private readonly listCard = viewChild.required('listCard', { read: ElementRef })
  private readonly nameField = viewChild.required('nameField', { read: ElementRef })

  readonly tokens = signal<ApiToken[]>([])
  readonly listState = signal<'loading' | 'loaded' | 'failed'>('loading')
  readonly created = signal<CreatedToken | null>(null)
  readonly createError = signal<string | null>(null)
  readonly creatingToken = signal(false)
  protected readonly longLifetime = API_TOKEN_LIFETIME_DAYS.reauthenticated

  readonly newLabel = new FormControl('', {
    nonNullable: true,
    validators: [Validators.maxLength(MAX_LABEL_LENGTH)],
  })
  readonly newPassword = new FormControl('', { nonNullable: true })

  protected readonly typedLabel = toSignal(this.newLabel.valueChanges, { initialValue: '' })
  private readonly typedPassword = toSignal(this.newPassword.valueChanges, { initialValue: '' })

  /** A token made with the password lasts a year; without, a week. */
  readonly lifetimeDays = computed(() =>
    this.typedPassword()
      ? API_TOKEN_LIFETIME_DAYS.reauthenticated
      : API_TOKEN_LIFETIME_DAYS.session,
  )

  protected readonly labelError = computed(() =>
    this.typedLabel().length > MAX_LABEL_LENGTH
      ? t('tokens.create.labelTooLong', { max: MAX_LABEL_LENGTH })
      : null,
  )

  ngOnInit(): void {
    this.reload()
  }

  reload(focusAfter: number | null = null): void {
    this.tokenService.list().subscribe({
      next: (tokens) => {
        this.tokens.set(tokens)
        this.listState.set('loaded')
        if (focusAfter !== null) {
          afterNextRender(() => this.focusInList(focusAfter), { injector: this.injector })
        }
      },
      error: () => {
        if (this.listState() !== 'loaded') {
          this.listState.set('failed')
        }
        this.toastService.error(t('tokens.errors.loadFailed'))
      },
    })
  }

  protected retry(): void {
    this.listState.set('loading')
    this.reload()
  }

  protected createdLabel(token: ApiToken): string {
    return this.when(token.created_at, 'tokens.list.createdAgo', 'tokens.list.createdOn')
  }

  protected lastUsedLabel(lastUsed: string): string {
    return this.when(lastUsed, 'tokens.list.usedAgo', 'tokens.list.usedOn')
  }

  /** "Créé il y a 3 j" within a month, "Créé le 12/08/2026" after: each reads as a sentence. */
  private when(iso: string, agoKey: string, onKey: string): string {
    const relative = formatRelativeTime(iso, activeLocale(), undefined, RELATIVE_OPTIONS)
    return /^\d/.test(relative)
      ? t(onKey, { date: formatLocalizedDate(iso, 'shortDate') })
      : t(agoKey, { when: relative })
  }

  createToken(): void {
    const label = this.newLabel.value.trim()
    if (!label || this.labelError() || this.creatingToken()) return
    const password = this.newPassword.value
    const days = this.lifetimeDays()
    this.creatingToken.set(true)
    this.createError.set(null)
    this.tokenService.create(label, password || null).subscribe({
      next: (result) => {
        this.creatingToken.set(false)
        this.created.set({ id: result.id, label, token: result.token, days })
        this.newLabel.setValue('')
        this.newPassword.setValue('')
        this.reload()
        this.toastService.success(t('tokens.created.toast'))
        // Emptying the name disables the focused button, so the focus moves to the token, shown
        // only once: a screen reader reads the card's name, the token, the warning, the button.
        afterNextRender(() => this.focusCard(this.revealCard()), { injector: this.injector })
      },
      error: (error: unknown) => {
        this.creatingToken.set(false)
        this.newPassword.setValue('')
        this.createError.set(createErrorMessage(error))
      },
    })
  }

  dismissCreatedToken(): void {
    this.created.set(null)
    afterNextRender(() => this.nameField().nativeElement.querySelector('input')?.focus(), {
      injector: this.injector,
    })
  }

  async revokeToken(token: ApiToken): Promise<void> {
    const confirmed = await this.confirmService.ask({
      heading: t('tokens.revoke.heading'),
      message: t('tokens.revoke.message', { label: token.label }),
      confirmLabel: t('tokens.revoke.confirm'),
      danger: true,
    })
    if (!confirmed) return
    const index = this.tokens().findIndex((each) => each.id === token.id)
    this.tokenService.revoke(token.id).subscribe({
      next: () => {
        if (this.created()?.id === token.id) {
          this.created.set(null)
        }
        this.toastService.success(t('tokens.revoke.done'))
        // The row's button goes with the row: the focus moves to the next one.
        this.reload(Math.max(index, 0))
      },
      error: () => this.toastService.error(t('tokens.errors.revokeFailed')),
    })
  }

  private focusInList(index: number): void {
    const card = this.listCard().nativeElement as HTMLElement
    const buttons = card.querySelectorAll<HTMLButtonElement>('.api-tokens-list__revoke button')
    const next = buttons[Math.min(index, buttons.length - 1)]
    if (next) {
      next.focus()
    } else {
      this.focusCard(this.listCard())
    }
  }

  /** `gbt-card` has no labelled region to focus, so its heading takes the focus, from script only. */
  private focusCard(host: ElementRef | undefined): void {
    const heading = (host?.nativeElement as HTMLElement | undefined)?.querySelector<HTMLElement>(
      'h2, h3',
    )
    if (heading) {
      heading.tabIndex = -1
      heading.focus()
    }
  }
}
