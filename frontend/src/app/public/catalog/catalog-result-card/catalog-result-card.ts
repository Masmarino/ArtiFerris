import { TranslocoPipe } from '@jsverse/transloco'
import { t } from '../../../shared/i18n/translator'
import { LocalizedDatePipe } from '../../../shared/i18n/localized-date'
import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core'
import { RouterLink } from '@angular/router'
import { Badge, Card, Icon } from '@masmarino/gabarit'
import { CopyableCommand } from '../../../shared/copyable-command/copyable-command'
import { formatRelativeDate, formatWeeklyDownloads } from '../../../shared/format'
import { CatalogEntry } from '../domain/catalog.entity'
import { ownerLink, packageLink, repositoryLink } from '../domain/catalog-links'
import { installCommand } from '../domain/install-command'

@Component({
  selector: 'app-catalog-result-card',
  standalone: true,
  imports: [TranslocoPipe, Badge, Card, CopyableCommand, LocalizedDatePipe, Icon, RouterLink],
  templateUrl: './catalog-result-card.html',
  styleUrl: './catalog-result-card.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class CatalogResultCard {
  readonly entry = input.required<CatalogEntry>()

  readonly formatLabel = computed(() => (this.entry().kind === 'docker' ? 'Docker' : 'npm'))
  readonly packagePath = computed(() => packageLink(this.entry()))
  readonly ownerPath = computed(() => ownerLink(this.entry().owner))
  readonly repositoryPath = computed(() => repositoryLink(this.entry()))
  readonly command = computed(() => installCommand(this.entry()))
  readonly updatedAgo = computed(() => formatRelativeDate(this.entry().updated_at))

  readonly weeklyDownloads = computed(() =>
    this.entry().downloads_7d > 0 ? formatWeeklyDownloads(this.entry().downloads_7d) : null,
  )

  readonly copyLabel = computed(() => t('package.copyInstallCommand', { name: this.entry().name }))
}
