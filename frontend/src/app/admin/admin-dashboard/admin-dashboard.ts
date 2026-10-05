import { activeLocale, t } from '../../shared/i18n/translator'
import { TranslocoPipe } from '@jsverse/transloco'
import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  OnInit,
  computed,
  inject,
  signal,
} from '@angular/core'
import { NgTemplateOutlet } from '@angular/common'
import { Subscription } from 'rxjs'
import { AdminMetricsService } from '../application/metrics.service'
import { AdminStats, MetricsSnapshot } from '../domain/metrics.entity'
import { AuditService } from '../application/audit.service'
import { AuditEntry } from '../domain/audit.entity'
import { auditEventLabel } from '../domain/audit-event-label'
import { RepositoriesService } from '../../repositories/application/repositories.service'
import { RepositorySummary } from '../../repositories/domain/repository.entity'
import { Alert } from '@masmarino/gabarit/alert'
import { Button } from '@masmarino/gabarit/button'
import { type ChartSeries } from '@masmarino/gabarit/chart'
import { DimensionCard, type DimensionRow } from '@masmarino/gabarit/dimension-card'
import { EmptyState } from '@masmarino/gabarit/empty-state'
import { formatRelativeTime } from '@masmarino/gabarit/format'
import { Icon } from '@masmarino/gabarit/icon'
import { LineChart } from '@masmarino/gabarit/line-chart'
import { ListCard } from '@masmarino/gabarit/list-card'
import { ListRow } from '@masmarino/gabarit/list-row'
import { PageLayout } from '@masmarino/gabarit/page-layout'
import { SegmentedControl, type SegmentedControlOption } from '@masmarino/gabarit/segmented-control'
import { Skeleton } from '@masmarino/gabarit/skeleton'
import { StatGrid } from '@masmarino/gabarit/stat-grid'
import { StatTile } from '@masmarino/gabarit/stat-tile'
import { formatBytes } from '../../shared/format'
import { exactTime } from '../../shared/row-date'
import { PageHeading } from '../../shared/page-heading/page-heading'

const RECENT_ACTIVITY_LIMIT = 10
const ACTIVITY_FETCH_LIMIT = 200
// Long enough to show a trend, short enough not to fill a quiet instance with empty bars.
const ACTIVITY_CHART_DAYS = 7
const BYTE_UNIT_KEYS = ['kb', 'mb', 'gb', 'tb']

/** The y-axis unit for the storage chart: Gabarit's line chart takes plain numbers. */
function byteUnit(bytes: number): { divisor: number; label: string } {
  let divisor = 1
  let label = t('format.bytes.b')
  for (const key of BYTE_UNIT_KEYS) {
    if (bytes < divisor * 1024) break
    divisor *= 1024
    label = t(`format.bytes.${key}`)
  }
  return { divisor, label }
}

interface DashboardTile {
  key: string
  label: string
  value: string
  hint: string | null
  icon: string
}

interface RecentRow {
  key: string
  label: string
  iso: string
  when: string
  title: string
}

type LoadState = 'loading' | 'loaded' | 'failed'

/**
 * The instance at a glance, laid out like FerrisGit's dashboard: its totals as tiles, then how
 * storage, users and repositories evolve over one chosen period. ArtiFerris adds the week's activity,
 * the repositories by format and the latest events. The server snapshots every hour and a line needs
 * two points: with fewer, the charts say there are not enough readings yet.
 */
@Component({
  selector: 'app-admin-dashboard',
  standalone: true,
  imports: [
    PageHeading,
    TranslocoPipe,
    NgTemplateOutlet,
    Alert,
    Button,
    DimensionCard,
    EmptyState,
    Icon,
    LineChart,
    ListCard,
    ListRow,
    PageLayout,
    SegmentedControl,
    Skeleton,
    StatGrid,
    StatTile,
  ],
  templateUrl: './admin-dashboard.html',
  styleUrl: './admin-dashboard.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class AdminDashboard implements OnInit {
  private readonly metricsService = inject(AdminMetricsService)
  private readonly auditService = inject(AuditService)
  private readonly repositoriesService = inject(RepositoriesService)

  readonly locale = activeLocale()
  readonly stats = signal<AdminStats | null>(null)
  readonly statsState = signal<LoadState>('loading')
  readonly activityEvents = signal<AuditEntry[]>([])
  readonly activityState = signal<LoadState>('loading')
  // More events than fetched: the older days are under-counted.
  readonly activityTruncated = signal(false)
  readonly repositories = signal<RepositorySummary[] | null>(null)
  readonly loadFailed = signal(false)

  /** Null until the first answer: skeletons show only then, not on a period change. */
  readonly history = signal<MetricsSnapshot[] | null>(null)
  readonly historyState = signal<LoadState>('loading')
  readonly historyDays = signal(30)
  protected readonly dayOptions: SegmentedControlOption<number>[] = [1, 3, 7, 30].map((days) => ({
    value: days,
    label: t(days === 1 ? 'admin.dashboard.days_one' : 'admin.dashboard.days_other', {
      count: days,
    }),
  }))
  protected readonly storagePane = { $implicit: 'storage' } as const
  protected readonly countsPane = { $implicit: 'counts' } as const

  private readonly latest = computed(() => {
    const history = this.history()
    return history && history.length > 0 ? history[history.length - 1] : null
  })

  protected readonly tiles = computed<DashboardTile[]>(() => {
    const stats = this.stats()
    const count = (value: number | undefined) => (value === undefined ? '—' : String(value))
    const latest = this.latest()
    return [
      {
        key: 'users',
        label: t('admin.dashboard.users'),
        value: count(stats?.total_users),
        hint: null,
        icon: 'user',
      },
      {
        key: 'repositories',
        label: t('admin.dashboard.repositories'),
        value: count(stats?.total_repositories),
        hint: null,
        icon: 'package',
      },
      {
        key: 'permissions',
        label: t('admin.dashboard.permissions'),
        value: count(stats?.total_active_permissions),
        hint: null,
        icon: 'shield',
      },
      {
        key: 'storage',
        label: t('admin.dashboard.storage'),
        value: latest ? formatBytes(latest.total_storage_bytes) : '—',
        hint: t(latest ? 'admin.dashboard.lastReading' : 'admin.dashboard.noReading'),
        icon: 'hard-drive',
      },
    ]
  })

  protected readonly hasEnoughReadings = computed(() => (this.history()?.length ?? 0) >= 2)

  protected readonly storageUnit = computed(() =>
    byteUnit(Math.max(0, ...(this.history() ?? []).map((h) => h.total_storage_bytes))),
  )

  readonly storageSeries = computed<ChartSeries<Date>[]>(() => {
    const { divisor, label } = this.storageUnit()
    return [
      {
        label: t('admin.dashboard.storageIn', { unit: label }),
        points: (this.history() ?? []).map((h) => ({
          x: new Date(h.recorded_at),
          y: h.total_storage_bytes / divisor,
          display: formatBytes(h.total_storage_bytes),
        })),
      },
    ]
  })

  readonly countsSeries = computed<ChartSeries<Date>[]>(() => {
    const history = this.history() ?? []
    return [
      {
        label: t('admin.dashboard.users'),
        points: history.map((h) => ({ x: new Date(h.recorded_at), y: h.total_users })),
      },
      {
        label: t('admin.dashboard.repositories'),
        points: history.map((h) => ({ x: new Date(h.recorded_at), y: h.total_repositories })),
      },
    ]
  })

  readonly activityChartData = computed<DimensionRow[]>(() => {
    const counts = new Map<string, number>()
    const days: string[] = []
    for (let i = ACTIVITY_CHART_DAYS - 1; i >= 0; i--) {
      const date = new Date()
      date.setUTCDate(date.getUTCDate() - i)
      const key = date.toISOString().slice(0, 10)
      days.push(key)
      counts.set(key, 0)
    }
    for (const event of this.activityEvents()) {
      const key = event.occurred_at.slice(0, 10)
      if (counts.has(key)) {
        counts.set(key, (counts.get(key) ?? 0) + 1)
      }
    }
    return days.map((day) => ({ label: formatDayLabel(day), value: counts.get(day) ?? 0 }))
  })

  readonly repositoriesByFormatChartData = computed<DimensionRow[]>(() => {
    const counts = new Map<string, number>()
    for (const repo of this.repositories() ?? []) {
      counts.set(repo.format, (counts.get(repo.format) ?? 0) + 1)
    }
    return Array.from(counts.entries()).map(([label, value]) => ({ label, value }))
  })

  readonly recentEvents = computed(() => this.activityEvents().slice(0, RECENT_ACTIVITY_LIMIT))
  protected readonly recentRows = computed<RecentRow[]>(() => {
    const now = new Date()
    return this.recentEvents().map((event) => ({
      key: event.occurred_at + event.event_type + event.aggregate_id,
      label: auditEventLabel(event.event_type),
      iso: event.occurred_at,
      when: formatRelativeTime(event.occurred_at, this.locale, now, {
        style: 'short',
        maxUnit: 'day',
        absoluteAfterDays: 30,
      }),
      title: exactTime(event.occurred_at),
    }))
  })

  private historyRequest: Subscription | null = null

  constructor() {
    inject(DestroyRef).onDestroy(() => this.historyRequest?.unsubscribe())
  }

  ngOnInit(): void {
    this.load()
  }

  retry(): void {
    this.load()
  }

  private load(): void {
    this.loadFailed.set(false)
    const failed = () => this.loadFailed.set(true)
    this.metricsService.stats().subscribe({
      next: (stats) => {
        this.stats.set(stats)
        this.statsState.set('loaded')
      },
      error: () => {
        this.statsState.set('failed')
        failed()
      },
    })

    const from = new Date()
    from.setUTCDate(from.getUTCDate() - (ACTIVITY_CHART_DAYS - 1))
    from.setUTCHours(0, 0, 0, 0)
    this.auditService.query({ from: from.toISOString(), limit: ACTIVITY_FETCH_LIMIT }).subscribe({
      next: ({ entries, next_cursor }) => {
        // Newest first, so one query serves the chart and the recent list.
        this.activityEvents.set(entries)
        this.activityTruncated.set(next_cursor !== null)
        this.activityState.set('loaded')
      },
      error: () => {
        if (this.activityState() !== 'loaded') this.activityState.set('failed')
        failed()
      },
    })

    this.repositoriesService.list().subscribe({
      next: (repos) => this.repositories.set(repos),
      error: failed,
    })

    this.loadHistory()
  }

  setHistoryDays(days: number): void {
    this.historyDays.set(days)
    this.loadHistory()
  }

  protected loadHistory(): void {
    // Only the latest period counts: a slower answer for an earlier one is dropped.
    this.historyRequest?.unsubscribe()
    this.historyState.set('loading')
    this.historyRequest = this.metricsService.history(this.historyDays()).subscribe({
      next: (history) => {
        this.history.set(history)
        this.historyState.set('loaded')
      },
      error: () => {
        this.historyState.set('failed')
        this.loadFailed.set(true)
      },
    })
  }
}

function formatDayLabel(isoDay: string): string {
  const [, month, day] = isoDay.split('-')
  return `${day}/${month}`
}
