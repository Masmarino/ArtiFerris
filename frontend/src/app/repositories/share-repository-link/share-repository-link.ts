import { DOCUMENT } from '@angular/common'
import { ChangeDetectionStrategy, Component, computed, inject, input } from '@angular/core'
import { RouterLink } from '@angular/router'
import { RepositorySummary } from '../domain/repository.entity'
import { CopyableCommand } from '../../shared/copyable-command/copyable-command'

interface PublicLink {
  path: string
  url: string
}

@Component({
  selector: 'app-share-repository-link',
  standalone: true,
  imports: [RouterLink, CopyableCommand],
  templateUrl: './share-repository-link.html',
  styleUrl: './share-repository-link.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ShareRepositoryLink {
  private readonly document = inject(DOCUMENT)

  readonly repository = input.required<RepositorySummary>()
  /** Whether the viewer can reach the Paramètres tab to change visibility — an admin only. */
  readonly canManageVisibility = input(false)

  readonly publicLink = computed<PublicLink | null>(() => {
    const path = this.repository().public_path
    return path ? { path, url: this.document.location.origin + path } : null
  })
}
