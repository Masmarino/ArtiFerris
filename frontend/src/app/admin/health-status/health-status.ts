import { activeLocale, t } from '../../shared/i18n/translator'
import { TranslocoPipe } from '@jsverse/transloco'
import { ChangeDetectionStrategy, Component, OnInit, computed, inject, signal } from '@angular/core'
import { Alert } from '@masmarino/gabarit/alert'
import { Badge, type BadgeVariant } from '@masmarino/gabarit/badge'
import { Button } from '@masmarino/gabarit/button'
import { Card } from '@masmarino/gabarit/card'
import { DescriptionList, type DescriptionListEntry } from '@masmarino/gabarit/description-list'
import { formatDuration, formatPercent } from '@masmarino/gabarit/format'
import { GaugeBar } from '@masmarino/gabarit/gauge-bar'
import { Icon } from '@masmarino/gabarit/icon'
import { PageLayout } from '@masmarino/gabarit/page-layout'
import { Panel } from '@masmarino/gabarit/panel'
import { Skeleton } from '@masmarino/gabarit/skeleton'
import { AdminMetricsService } from '../application/metrics.service'
import { HealthStatus } from '../domain/metrics.entity'
import { formatBytes } from '../../shared/format'
import { exactTime } from '../../shared/row-date'
import { PageHeading } from '../../shared/page-heading/page-heading'
import { ToastService } from '../../shared/toast.service'

interface StatusBadge {
  label: string
  variant: BadgeVariant
  icon: string
}

interface Gauge {
  label: string
  value: number
  max: number
  formatted: string
}

interface HealthCard {
  key: string
  heading: string
  icon: string
  status: StatusBadge
  detail: string | null
  facts: DescriptionListEntry[]
  gauge: Gauge | null
}

const UP_ICON = 'circle-check'
const DOWN_ICON = 'circle-x'

const componentStatus = (status: 'up' | 'down', upKey: string): StatusBadge =>
  status === 'up'
    ? { label: t(upKey), variant: 'success', icon: UP_ICON }
    : { label: t('admin.health.down'), variant: 'error', icon: DOWN_ICON }

/**
 * The instance's services, laid out like FerrisGit's: an overall badge and the time of the check in
 * the header, a card per service with its state, gauge and figures, and the uptime on the side.
 * The page checks once on opening and again on "Actualiser".
 */
@Component({
  selector: 'app-health-status',
  standalone: true,
  imports: [
    PageHeading,
    TranslocoPipe,
    Alert,
    Badge,
    Button,
    Card,
    DescriptionList,
    GaugeBar,
    Icon,
    PageLayout,
    Panel,
    Skeleton,
  ],
  templateUrl: './health-status.html',
  styleUrl: './health-status.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class HealthStatusPage implements OnInit {
  private readonly metricsService = inject(AdminMetricsService)
  private readonly toast = inject(ToastService)

  readonly status = signal<HealthStatus | null>(null)
  // Not a service reported down, which comes in a successful answer: the request itself failed.
  readonly loadError = signal(false)
  readonly checking = signal(false)
  private readonly checkedAt = signal<Date | null>(null)
  protected readonly skeletonCards = [0, 1]
  protected readonly gaugeThresholds = { warning: 80, critical: 95 }

  protected readonly cards = computed<HealthCard[]>(() => {
    const health = this.status()
    if (!health) return []
    const { database, storage } = health
    const up = (status: 'up' | 'down') => status === 'up'
    return [
      {
        key: 'database',
        heading: t('admin.health.database'),
        icon: 'database',
        status: componentStatus(database.status, 'admin.health.upFeminine'),
        detail: database.detail,
        facts: up(database.status)
          ? [
              {
                term: t('admin.health.version'),
                value: database.server_version
                  ? t('admin.health.postgres', { version: database.server_version })
                  : t('admin.health.unknownVersion'),
              },
              {
                term: t('admin.health.responseTime'),
                value: t('admin.health.milliseconds', { count: database.response_time_ms }),
              },
            ]
          : [],
        gauge: up(database.status)
          ? {
              label: t('admin.health.connections'),
              value: database.active_connections,
              max: database.max_connections,
              formatted: t('admin.health.ratio', {
                value: database.active_connections,
                max: database.max_connections,
              }),
            }
          : null,
      },
      {
        key: 'storage',
        heading: t('admin.health.storage'),
        icon: 'hard-drive',
        status: componentStatus(storage.status, 'admin.health.up'),
        detail: storage.detail,
        facts: up(storage.status)
          ? [
              { term: t('admin.health.used'), value: formatBytes(storage.used_bytes) },
              { term: t('admin.health.free'), value: formatBytes(storage.free_bytes) },
              { term: t('admin.health.total'), value: formatBytes(storage.total_bytes) },
            ]
          : [],
        gauge: up(storage.status)
          ? {
              label: t('admin.health.usedSpace'),
              value: storage.used_bytes,
              max: storage.total_bytes,
              formatted: t('admin.health.storageRatio', {
                used: formatBytes(storage.used_bytes),
                total: formatBytes(storage.total_bytes),
                percent: formatPercent(
                  storage.total_bytes > 0 ? storage.used_bytes / storage.total_bytes : 0,
                  activeLocale(),
                ),
              }),
            }
          : null,
      },
    ]
  })

  protected readonly overall = computed<StatusBadge | null>(() => {
    if (this.loadError()) {
      return { label: t('admin.health.unreachable'), variant: 'error', icon: DOWN_ICON }
    }
    const health = this.status()
    if (!health) return null
    return health.database.status === 'up' && health.storage.status === 'up'
      ? { label: t('admin.health.allUp'), variant: 'success', icon: UP_ICON }
      : { label: t('admin.health.degraded'), variant: 'error', icon: 'alert-triangle' }
  })

  protected readonly uptime = computed(() => {
    const health = this.status()
    const checkedAt = this.checkedAt()
    if (!health || !checkedAt) return null
    const startedAt = new Date(checkedAt.getTime() - health.uptime_seconds * 1000).toISOString()
    return {
      value: formatDuration(health.uptime_seconds * 1000, activeLocale(), { days: true }),
      startedAt: t('admin.health.startedAt', { date: exactTime(startedAt) }),
    }
  })

  /** Hidden after a failed check, where an older time would look current. */
  protected readonly checkedAtLabel = computed(() => {
    const checkedAt = this.checkedAt()
    if (!checkedAt || this.loadError()) return null
    return new Intl.DateTimeFormat(activeLocale(), {
      hour: '2-digit',
      minute: '2-digit',
      second: '2-digit',
    }).format(checkedAt)
  })

  ngOnInit(): void {
    this.refresh()
  }

  refresh(): void {
    this.checking.set(true)
    this.metricsService.health().subscribe({
      next: (status) => {
        this.status.set(status)
        this.checkedAt.set(new Date())
        this.loadError.set(false)
        this.checking.set(false)
      },
      error: () => {
        this.loadError.set(true)
        this.checking.set(false)
        this.toast.error(t('admin.health.loadFailed'))
      },
    })
  }
}
