import { activeLocale, t } from '../../shared/i18n/translator'
import { TranslocoPipe } from '@jsverse/transloco'
import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  computed,
  effect,
  inject,
  input,
  signal,
} from '@angular/core'
import { formatLocalizedDate } from '../../shared/i18n/localized-date'
import { HttpErrorResponse } from '@angular/common/http'
import { Subscription, last, tap } from 'rxjs'
import { Button } from '@masmarino/gabarit/button'
import { Card } from '@masmarino/gabarit/card'
import { DimensionCard, type DimensionRow } from '@masmarino/gabarit/dimension-card'
import { EmptyState } from '@masmarino/gabarit/empty-state'
import { Spinner } from '@masmarino/gabarit/spinner'
import { Table, TableColumn } from '@masmarino/gabarit/table'
import { AuditService } from '../application/audit.service'
import { AuditEntry, BlockedAccount } from '../domain/audit.entity'
import { describeBlockedAccount } from '../domain/blocked-account'
import { auditEventDetails, auditEventLabel, auditPayload } from '../domain/audit-event-label'
import { UsersService } from '../../users/application/users.service'
import { OrganizationMembersService } from '../application/organization-members.service'
import { RepositoriesService } from '../../repositories/application/repositories.service'
import { AUDIT_EXPORT_MAX_ROWS, collectAuditPages, isPartialExport } from '../domain/audit-export'
import { csvBlob, toCsv } from '../../shared/csv'
import { downloadBlob } from '../../shared/download'
import { ConfirmService } from '../../shared/confirm.service'
import { ToastService } from '../../shared/toast.service'

interface SecurityLogRow {
  occurred_at: string
  event_type: string
  actor: string
  details: string
}

interface EventTypeCount {
  event_type: string
  count: number
}

function unlockFailureMessage(error: unknown, username: string): string {
  switch (error instanceof HttpErrorResponse ? error.status : null) {
    case 403:
      return t('admin.securityLog.errors.forbidden', { username })
    case 404:
      return t('admin.securityLog.errors.notFound', { username })
    default:
      return t('admin.securityLog.errors.unlockFailed', { username })
  }
}

@Component({
  selector: 'app-security-log',
  standalone: true,
  imports: [TranslocoPipe, Table, DimensionCard, Button, Card, EmptyState, Spinner],
  templateUrl: './security-log.html',
  styleUrl: './security-log.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class SecurityLog {
  private readonly auditService = inject(AuditService)
  private readonly usersService = inject(UsersService)
  private readonly organizationMembersService = inject(OrganizationMembersService)
  private readonly repositoriesService = inject(RepositoriesService)
  private readonly confirmService = inject(ConfirmService)
  private readonly toastService = inject(ToastService)
  private exportRun: Subscription | null = null

  /** Scopes the view to one organization and hides the blocked-accounts panel. */
  readonly organizationId = input<string | undefined>(undefined)

  private readonly entries = signal<AuditEntry[]>([])
  private readonly usernamesById = signal<Map<string, string>>(new Map())
  private readonly repositoryNamesById = signal<Map<string, string>>(new Map())
  readonly blockedAccounts = signal<BlockedAccount[]>([])
  readonly nextCursor = signal<string | null>(null)
  readonly loading = signal(true)
  readonly loadFailed = signal(false)
  readonly loadingMore = signal(false)
  readonly loadMoreFailed = signal(false)

  readonly unlocking = signal(false)

  readonly exporting = signal(false)
  readonly exportedCount = signal(0)
  readonly exportFailed = signal(false)
  readonly exportPartial = signal(false)
  readonly exportMaxRows = AUDIT_EXPORT_MAX_ROWS

  readonly rows = computed<SecurityLogRow[]>(() => this.toRows(this.entries()))

  readonly summary = computed<EventTypeCount[]>(() => {
    const counts = new Map<string, number>()
    for (const entry of this.entries()) {
      const label = auditEventLabel(entry.event_type)
      counts.set(label, (counts.get(label) ?? 0) + 1)
    }
    return Array.from(counts.entries())
      .map(([event_type, count]) => ({ event_type, count }))
      .sort((a, b) => b.count - a.count)
  })

  readonly locale = activeLocale()

  readonly summaryChartData = computed<DimensionRow[]>(() =>
    this.summary().map((item) => ({ label: item.event_type, value: item.count })),
  )

  readonly blockedAccountRows = computed(() =>
    this.blockedAccounts().map((account) => ({
      key: account.username,
      ...describeBlockedAccount(account),
      remaining: this.formatRemainingTime(account.remaining_seconds),
    })),
  )

  readonly columns: TableColumn<SecurityLogRow>[] = [
    {
      key: 'occurred_at',
      label: t('common.date'),
      format: (r) => formatLocalizedDate(r.occurred_at, 'short'),
    },
    { key: 'event_type', label: t('admin.auditLog.columns.event') },
    { key: 'actor', label: t('admin.securityLog.columns.user') },
    { key: 'details', label: t('admin.auditLog.columns.details') },
  ]
  readonly rowId = (r: SecurityLogRow): string =>
    `${r.occurred_at}|${r.event_type}|${r.actor}|${r.details}`

  // effect, not ngOnInit: this component is reused across organizations.
  constructor() {
    inject(DestroyRef).onDestroy(() => this.exportRun?.unsubscribe())
    effect(() => {
      const organizationId = this.organizationId()
      this.cancelExport()
      this.exportFailed.set(false)
      this.exportPartial.set(false)
      this.entries.set([])
      this.nextCursor.set(null)
      this.loading.set(true)
      this.loadFailed.set(false)
      this.loadingMore.set(false)
      this.loadMoreFailed.set(false)
      const stillCurrent = () => this.organizationId() === organizationId

      this.auditService
        .query({ aggregate_type: 'Security', organization_id: organizationId })
        .subscribe({
          next: (page) => {
            if (!stillCurrent()) {
              return
            }
            this.entries.set(page.entries)
            this.nextCursor.set(page.next_cursor)
            this.loading.set(false)
          },
          error: () => {
            if (!stillCurrent()) {
              return
            }
            this.loadFailed.set(true)
            this.loading.set(false)
          },
        })

      if (organizationId) {
        this.organizationMembersService.list(organizationId).subscribe({
          next: (members) => {
            if (!stillCurrent()) {
              return
            }
            this.usernamesById.set(new Map(members.map((m) => [m.id, m.username])))
          },
          error: () => {
            // Actor names are cosmetic: fall back to ids on failure.
          },
        })
      } else {
        this.usersService.list().subscribe({
          next: (users) => {
            if (!stillCurrent()) {
              return
            }
            this.usernamesById.set(new Map(users.map((u) => [u.id, u.username])))
          },
          error: () => {
            // Actor names are cosmetic: fall back to ids on failure.
          },
        })
        this.loadBlockedAccounts(organizationId)
      }

      this.repositoriesService.list().subscribe({
        next: (repos) => {
          if (!stillCurrent()) {
            return
          }
          this.repositoryNamesById.set(new Map(repos.map((r) => [r.id, r.name])))
        },
        error: () => {
          // Repository names are cosmetic: fall back to ids on failure.
        },
      })
    })
  }

  private loadBlockedAccounts(organizationId: string | undefined): void {
    this.auditService.blockedAccounts().subscribe({
      next: (accounts) => {
        if (this.organizationId() === organizationId) {
          this.blockedAccounts.set(accounts)
        }
      },
      error: () => {
        // Best effort: a failed lookup leaves the panel empty.
      },
    })
  }

  async unlock(username: string): Promise<void> {
    if (this.unlocking()) {
      return
    }
    const confirmed = await this.confirmService.ask({
      heading: t('admin.securityLog.unlockHeading'),
      message: t('admin.securityLog.unlockMessage', { username }),
      confirmLabel: t('admin.securityLog.unlock'),
    })
    if (!confirmed) {
      return
    }
    const organizationId = this.organizationId()
    this.unlocking.set(true)
    this.auditService.unlockUsername(username).subscribe({
      next: () => {
        this.unlocking.set(false)
        this.toastService.success(t('admin.securityLog.unlocked', { username }))
        this.loadBlockedAccounts(organizationId)
      },
      error: (err: unknown) => {
        this.unlocking.set(false)
        this.toastService.error(unlockFailureMessage(err, username))
      },
    })
  }

  loadMore(): void {
    const cursor = this.nextCursor()
    if (!cursor || this.loadingMore()) {
      return
    }
    const organizationId = this.organizationId()
    this.loadingMore.set(true)
    this.loadMoreFailed.set(false)
    this.auditService
      .query({ aggregate_type: 'Security', organization_id: organizationId, cursor })
      .subscribe({
        next: (page) => {
          if (this.organizationId() !== organizationId) {
            return
          }
          this.entries.update((entries) => [...entries, ...page.entries])
          this.nextCursor.set(page.next_cursor)
          this.loadingMore.set(false)
        },
        error: () => {
          if (this.organizationId() !== organizationId) {
            return
          }
          this.loadMoreFailed.set(true)
          this.loadingMore.set(false)
        },
      })
  }

  formatRemainingTime(seconds: number): string {
    const minutes = Math.ceil(seconds / 60)
    return minutes <= 1
      ? t('admin.securityLog.lessThanMinute')
      : t('admin.securityLog.minutes', { count: minutes })
  }

  downloadCsv(): void {
    if (this.exporting()) {
      return
    }
    this.exportFailed.set(false)
    this.exportPartial.set(false)
    const cursor = this.nextCursor()
    if (!cursor) {
      this.saveCsv(this.entries())
      return
    }
    const organizationId = this.organizationId()
    this.exporting.set(true)
    this.exportedCount.set(this.entries().length)
    this.exportRun = collectAuditPages(this.entries(), cursor, (next) =>
      this.auditService.query({
        aggregate_type: 'Security',
        organization_id: organizationId,
        cursor: next,
      }),
    )
      .pipe(
        tap((state) => this.exportedCount.set(state.entries.length)),
        last(),
      )
      .subscribe({
        next: (state) => {
          this.exporting.set(false)
          this.exportPartial.set(isPartialExport(state))
          this.saveCsv(state.entries.slice(0, AUDIT_EXPORT_MAX_ROWS))
        },
        error: () => {
          this.exporting.set(false)
          this.exportFailed.set(true)
        },
      })
  }

  cancelExport(): void {
    this.exportRun?.unsubscribe()
    this.exportRun = null
    this.exporting.set(false)
  }

  private saveCsv(entries: AuditEntry[]): void {
    const csv = toCsv(this.toRows(entries), this.columns)
    downloadBlob(csvBlob(csv), `artiferris-security-${new Date().toISOString().slice(0, 10)}.csv`)
  }

  private toRows(entries: AuditEntry[]): SecurityLogRow[] {
    return entries.map((entry) => ({
      occurred_at: entry.occurred_at,
      event_type: auditEventLabel(entry.event_type),
      actor: this.actorLabel(entry),
      details: this.detailsLabel(entry),
    }))
  }

  private actorLabel(entry: AuditEntry): string {
    const payload = auditPayload(entry)
    if (typeof payload['username'] === 'string') {
      return payload['username']
    }
    if (entry.actor_id) {
      return this.usernamesById().get(entry.actor_id) ?? entry.actor_id
    }
    return '—'
  }

  private detailsLabel(entry: AuditEntry): string {
    const payload = auditPayload(entry)
    switch (entry.event_type) {
      case 'LoginFailed':
      case 'PasswordChangeFailed':
        return typeof payload['ip'] === 'string'
          ? t('admin.securityLog.from', { ip: payload['ip'] })
          : ''
      case 'AccessDenied': {
        const repositoryId = payload['repository_id']
        const repositoryName =
          typeof repositoryId === 'string'
            ? (this.repositoryNamesById().get(repositoryId) ?? repositoryId)
            : '?'
        const action = typeof payload['action'] === 'string' ? payload['action'] : '?'
        return t('admin.securityLog.accessDenied', { action, repository: repositoryName })
      }
      default:
        return auditEventDetails(entry)
    }
  }
}
