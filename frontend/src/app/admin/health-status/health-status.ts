import { t } from '../../shared/i18n/translator'
import { TranslocoPipe } from '@jsverse/transloco'
import {
  ChangeDetectionStrategy,
  Component,
  OnInit,
  TemplateRef,
  computed,
  inject,
  signal,
  viewChild,
} from '@angular/core'
import { catchError, of } from 'rxjs'
import { AdminMetricsService } from '../application/metrics.service'
import { HealthStatus } from '../domain/metrics.entity'
import {
  Badge,
  Card,
  DescriptionList,
  GaugeBar,
  type DescriptionListEntry,
} from '@masmarino/gabarit'
import { formatBytes } from '../../shared/format'

@Component({
  selector: 'app-health-status',
  standalone: true,
  imports: [TranslocoPipe, Badge, Card, DescriptionList, GaugeBar],
  templateUrl: './health-status.html',
  styleUrl: './health-status.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class HealthStatusPage implements OnInit {
  private readonly metricsService = inject(AdminMetricsService)

  readonly status = signal<HealthStatus | null>(null)
  readonly loadError = signal(false)

  private readonly dbStatusValue = viewChild<TemplateRef<unknown>>('dbStatusValue')
  private readonly storageStatusValue = viewChild<TemplateRef<unknown>>('storageStatusValue')

  readonly databaseItems = computed<DescriptionListEntry[]>(() => {
    const s = this.status()
    const statusValue = this.dbStatusValue()
    if (!s || !statusValue) return []
    const items: DescriptionListEntry[] = [
      { term: t('admin.health.status'), value: statusValue },
      {
        term: t('admin.health.responseTime'),
        value: t('admin.health.milliseconds', { count: s.database.response_time_ms }),
      },
    ]
    if (s.database.server_version) {
      items.push({ term: t('admin.health.postgresVersion'), value: s.database.server_version })
    }
    return items
  })

  readonly storageItems = computed<DescriptionListEntry[]>(() => {
    const s = this.status()
    const statusValue = this.storageStatusValue()
    if (!s || !statusValue) return []
    return [
      { term: t('admin.health.status'), value: statusValue },
      { term: t('admin.health.freeSpace'), value: formatBytes(s.storage.free_bytes) },
    ]
  })

  readonly serverItems = computed<DescriptionListEntry[]>(() => {
    const s = this.status()
    if (!s) return []
    return [{ term: t('admin.health.uptime'), value: this.formatUptime(s.uptime_seconds) }]
  })

  ngOnInit(): void {
    this.metricsService
      .health()
      .pipe(
        catchError(() => {
          this.loadError.set(true)
          return of(null)
        }),
      )
      .subscribe((status) => {
        if (status) this.status.set(status)
      })
  }

  readonly formatByteRatio = (value: number, max: number): string =>
    `${formatBytes(value)} / ${formatBytes(max)}`

  readonly formatRatio = (value: number, max: number): string => `${value} / ${max}`

  formatUptime(seconds: number): string {
    const days = Math.floor(seconds / 86400)
    const hours = Math.floor((seconds % 86400) / 3600)
    const minutes = Math.floor((seconds % 3600) / 60)
    const parts: string[] = []
    if (days > 0) {
      parts.push(t('admin.health.days', { count: days }))
    }
    if (days > 0 || hours > 0) {
      parts.push(t('admin.health.hours', { count: hours }))
    }
    parts.push(t('admin.health.minutes', { count: minutes }))
    return parts.join(' ')
  }
}
