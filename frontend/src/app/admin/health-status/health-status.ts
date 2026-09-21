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
  imports: [Badge, Card, DescriptionList, GaugeBar],
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
      { term: 'Statut', value: statusValue },
      { term: 'Temps de réponse', value: `${s.database.response_time_ms} ms` },
    ]
    if (s.database.server_version) {
      items.push({ term: 'Version PostgreSQL', value: s.database.server_version })
    }
    return items
  })

  readonly storageItems = computed<DescriptionListEntry[]>(() => {
    const s = this.status()
    const statusValue = this.storageStatusValue()
    if (!s || !statusValue) return []
    return [
      { term: 'Statut', value: statusValue },
      { term: 'Espace libre', value: formatBytes(s.storage.free_bytes) },
    ]
  })

  readonly serverItems = computed<DescriptionListEntry[]>(() => {
    const s = this.status()
    if (!s) return []
    return [{ term: 'Démarré depuis', value: this.formatUptime(s.uptime_seconds) }]
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
      parts.push(`${days} j`)
    }
    if (days > 0 || hours > 0) {
      parts.push(`${hours} h`)
    }
    parts.push(`${minutes} min`)
    return parts.join(' ')
  }
}
