import { activeLocale, t } from '../../shared/i18n/translator'
import { TranslocoPipe } from '@jsverse/transloco'
import { ChangeDetectionStrategy, Component, computed, inject, input } from '@angular/core'
import { rxResource } from '@angular/core/rxjs-interop'
import { Button } from '@masmarino/gabarit/button'
import { Card } from '@masmarino/gabarit/card'
import { DimensionCard, DimensionRow } from '@masmarino/gabarit/dimension-card'
import { GaugeBar } from '@masmarino/gabarit/gauge-bar'
import { Table, TableColumn } from '@masmarino/gabarit/table'
import { AdminMetricsService } from '../application/metrics.service'
import { RepositoryUsage } from '../domain/metrics.entity'
import { formatBytes } from '../../shared/format'

// Past this many, the rest folds into one "Autres" bar.
const CHART_TOP_N = 15

@Component({
  selector: 'app-usage-metrics',
  standalone: true,
  imports: [TranslocoPipe, Table, DimensionCard, Card, GaugeBar, Button],
  templateUrl: './usage-metrics.html',
  styleUrl: './usage-metrics.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class UsageMetrics {
  private readonly metricsService = inject(AdminMetricsService)

  readonly organizationId = input<string | undefined>(undefined)

  readonly locale = activeLocale()

  // Switching organization drops the previous answer and cancels its request.
  private readonly resource = rxResource({
    params: () => ({ organizationId: this.organizationId() }),
    stream: ({ params }) => this.metricsService.usage(params.organizationId),
  })

  readonly usages = computed<RepositoryUsage[]>(() =>
    !this.resource.isLoading() && this.resource.hasValue() ? this.resource.value() : [],
  )
  readonly failed = computed(
    () => !this.resource.isLoading() && this.resource.error() !== undefined,
  )
  readonly formatBytes = formatBytes
  readonly quotaFormatter = (value: number, max: number) =>
    `${formatBytes(value)} / ${formatBytes(max)}`

  readonly columns: TableColumn<RepositoryUsage>[] = [
    { key: 'name', label: t('admin.usage.repository') },
    { key: 'used_bytes', label: t('admin.usage.usedBytes') },
  ]
  readonly rowId = (u: RepositoryUsage): string => u.repository_id

  // Unlimited repositories have no threshold to warn about.
  readonly quotasInUse = computed(() => this.usages().filter((u) => u.quota_bytes != null))

  readonly chartData = computed<DimensionRow[]>(() => {
    const sorted = [...this.usages()].sort((a, b) => b.used_bytes - a.used_bytes)
    const top = sorted.slice(0, CHART_TOP_N)
    const rest = sorted.slice(CHART_TOP_N)
    const data = top.map((u) => ({
      label: u.name,
      value: u.used_bytes,
      display: formatBytes(u.used_bytes),
    }))
    if (rest.length > 0) {
      const total = rest.reduce((sum, u) => sum + u.used_bytes, 0)
      data.push({
        label: t('admin.usage.others', { count: rest.length }),
        value: total,
        display: formatBytes(total),
      })
    }
    return data
  })

  retry(): void {
    this.resource.reload()
  }
}
