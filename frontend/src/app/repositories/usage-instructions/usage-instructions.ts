import { TranslocoPipe } from '@jsverse/transloco'
import { t } from '../../shared/i18n/translator'
import { DOCUMENT } from '@angular/common'
import { ChangeDetectionStrategy, Component, computed, inject, input } from '@angular/core'
import { RouterLink } from '@angular/router'
import { RepositorySummary } from '../domain/repository.entity'

@Component({
  selector: 'app-usage-instructions',
  standalone: true,
  imports: [TranslocoPipe, RouterLink],
  templateUrl: './usage-instructions.html',
  styleUrl: './usage-instructions.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class UsageInstructions {
  private readonly document = inject(DOCUMENT)

  readonly repository = input.required<RepositorySummary>()
  readonly apiTokenLink = input('/account')
  readonly apiTokenLinkLabel = input(t('repositories.usage.createApiToken'))

  readonly host = this.document.location.host
  readonly origin = this.document.location.origin
  readonly isHosted = computed(() => this.repository().repo_type === 'hosted')
  /** A personal repository lives under `u/{owner}/{name}`, an organization one under `{name}`. */
  readonly repositoryPath = computed(() => {
    const repository = this.repository()
    return repository.owner_is_personal
      ? `u/${repository.owner_name}/${repository.name}`
      : repository.name
  })
}
