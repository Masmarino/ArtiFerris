import {
  ChangeDetectionStrategy,
  Component,
  computed,
  effect,
  inject,
  input,
  signal,
} from '@angular/core'
import { DatePipe } from '@angular/common'
import { Button, Card, EmptyState, Tooltip } from '@masmarino/gabarit'
import { AdminApiTokensService } from '../application/admin-api-tokens.service'
import { ADMIN_TOKEN_PAGE_LIMIT, AdminApiToken } from '../domain/admin-api-token.entity'
import { ConfirmService } from '../../shared/confirm.service'
import { ToastService } from '../../shared/toast.service'

@Component({
  selector: 'app-api-tokens-admin',
  standalone: true,
  imports: [Button, Card, DatePipe, EmptyState, Tooltip],
  templateUrl: './api-tokens.html',
  styleUrl: './api-tokens.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ApiTokensAdmin {
  private readonly tokensService = inject(AdminApiTokensService)
  private readonly confirmService = inject(ConfirmService)
  private readonly toastService = inject(ToastService)

  /** Set only when embedded in an organization's own admin page — scopes the list to it. */
  readonly organizationId = input<string | undefined>(undefined)

  private readonly tokens = signal<AdminApiToken[]>([])
  readonly loadFailed = signal(false)

  readonly activeTokens = computed(() => this.tokens().filter((t) => !t.revoked_at))
  readonly revokedTokens = computed(() => this.tokens().filter((t) => t.revoked_at))
  readonly pageLimit = ADMIN_TOKEN_PAGE_LIMIT
  readonly listTruncated = computed(() => this.tokens().length >= ADMIN_TOKEN_PAGE_LIMIT)

  // effect(), not ngOnInit — this component is reused across organizations on the same route.
  constructor() {
    effect(() => {
      this.organizationId()
      this.tokens.set([])
      this.reload()
    })
  }

  private reload(): void {
    const organizationId = this.organizationId()
    const stillCurrent = () => this.organizationId() === organizationId
    this.loadFailed.set(false)
    this.tokensService.list(organizationId).subscribe({
      next: (tokens) => {
        if (stillCurrent()) {
          this.tokens.set(tokens)
        }
      },
      error: () => {
        if (stillCurrent()) {
          this.loadFailed.set(true)
        }
      },
    })
  }

  async revoke(token: AdminApiToken): Promise<void> {
    const confirmed = await this.confirmService.ask({
      heading: 'Révoquer le jeton',
      message: `Révoquer le jeton « ${token.label} » de ${token.username} ?`,
      confirmLabel: 'Révoquer',
      danger: true,
    })
    if (!confirmed) return
    this.tokensService.revoke(token.id).subscribe({
      next: () => {
        this.reload()
        this.toastService.success(`Jeton « ${token.label} » révoqué.`)
      },
      error: () => this.toastService.error(`Échec de la révocation du jeton « ${token.label} ».`),
    })
  }
}
