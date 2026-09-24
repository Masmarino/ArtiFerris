import {
  ChangeDetectionStrategy,
  Component,
  LOCALE_ID,
  DestroyRef,
  computed,
  effect,
  inject,
  input,
  signal,
} from '@angular/core'
import { DatePipe } from '@angular/common'
import { Subscription, last, tap } from 'rxjs'
import {
  Button,
  Card,
  DimensionCard,
  type DimensionRow,
  EmptyState,
  Spinner,
  Table,
  TableColumn,
} from '@masmarino/gabarit'
import { AuditService } from '../application/audit.service'
import { AuditEntry } from '../domain/audit.entity'
import { auditEventDetails, auditEventLabel } from '../domain/audit-event-label'
import { AUDIT_EXPORT_MAX_ROWS, collectAuditPages, isPartialExport } from '../domain/audit-export'
import { csvBlob, toCsv } from '../../shared/csv'
import { downloadBlob } from '../../shared/download'

interface AuditCsvRow {
  occurred_at: string
  aggregate_type: string
  aggregate_id: string
  event_type: string
  actor_id: string | null
  payload: string
}

interface AuditLogRow {
  occurred_at: string
  aggregate_type: string
  aggregate_id: string
  event_type: string
  actor_id: string | null
  details: string
}

@Component({
  selector: 'app-audit-log',
  standalone: true,
  imports: [Table, DimensionCard, Button, Card, EmptyState, Spinner],
  providers: [DatePipe],
  templateUrl: './audit-log.html',
  styleUrl: './audit-log.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class AuditLog {
  private readonly auditService = inject(AuditService)
  private readonly datePipe = inject(DatePipe)
  private exportRun: Subscription | null = null

  /** Set only when embedded in an organization's own admin page — scopes the query to it. */
  readonly organizationId = input<string | undefined>(undefined)

  readonly entries = signal<AuditEntry[]>([])
  readonly nextCursor = signal<string | null>(null)
  readonly loading = signal(true)
  readonly loadFailed = signal(false)
  readonly loadingMore = signal(false)
  readonly loadMoreFailed = signal(false)

  readonly exporting = signal(false)
  readonly exportedCount = signal(0)
  readonly exportFailed = signal(false)
  readonly exportPartial = signal(false)
  readonly exportMaxRows = AUDIT_EXPORT_MAX_ROWS

  readonly rows = computed<AuditLogRow[]>(() =>
    this.entries().map((entry) => ({
      occurred_at: entry.occurred_at,
      aggregate_type: entry.aggregate_type,
      aggregate_id: entry.aggregate_id,
      event_type: auditEventLabel(entry.event_type),
      actor_id: entry.actor_id,
      details: auditEventDetails(entry),
    })),
  )

  private readonly csvColumns: { key: keyof AuditCsvRow; label: string }[] = [
    { key: 'occurred_at', label: 'Date' },
    { key: 'aggregate_type', label: 'Type' },
    { key: 'aggregate_id', label: 'Identifiant' },
    { key: 'event_type', label: 'Événement' },
    { key: 'actor_id', label: 'Acteur' },
    { key: 'payload', label: 'Détails' },
  ]

  readonly columns: TableColumn<AuditLogRow>[] = [
    {
      key: 'occurred_at',
      label: 'Date',
      format: (e) => this.datePipe.transform(e.occurred_at, 'short') ?? '',
    },
    { key: 'aggregate_type', label: 'Type' },
    { key: 'event_type', label: 'Événement' },
    { key: 'actor_id', label: 'Acteur' },
    { key: 'details', label: 'Détails' },
  ]
  readonly rowId = (r: AuditLogRow): string =>
    `${r.occurred_at}|${r.aggregate_id}|${r.event_type}|${r.actor_id}|${r.details}`

  readonly locale = inject(LOCALE_ID)

  readonly summaryChartData = computed<DimensionRow[]>(() => {
    const counts = new Map<string, number>()
    for (const entry of this.entries()) {
      counts.set(entry.aggregate_type, (counts.get(entry.aggregate_type) ?? 0) + 1)
    }
    return Array.from(counts.entries())
      .map(([label, value]) => ({ label, value }))
      .sort((a, b) => b.value - a.value)
  })

  // effect(), not ngOnInit — this component is reused across organizations on the same route.
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
      // Security events have their own screen, excluded server-side.
      this.auditService
        .query({ exclude_aggregate_type: 'Security', organization_id: organizationId })
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
      .query({ exclude_aggregate_type: 'Security', organization_id: organizationId, cursor })
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

  /** Fetches the pages not loaded yet (up to the row bound), then saves the CSV. */
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
        exclude_aggregate_type: 'Security',
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
    const rows: AuditCsvRow[] = entries.map((entry) => ({
      occurred_at: entry.occurred_at,
      aggregate_type: entry.aggregate_type,
      aggregate_id: entry.aggregate_id,
      event_type: entry.event_type,
      actor_id: entry.actor_id,
      payload: JSON.stringify(entry.payload),
    }))
    const csv = toCsv(rows, this.csvColumns)
    downloadBlob(csvBlob(csv), `artiferris-audit-${new Date().toISOString().slice(0, 10)}.csv`)
  }
}
