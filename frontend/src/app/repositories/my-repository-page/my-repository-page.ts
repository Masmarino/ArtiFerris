import { t } from '../../shared/i18n/translator'
import { TranslocoPipe } from '@jsverse/transloco'
import { ChangeDetectionStrategy, Component, OnInit, inject, signal } from '@angular/core'
import { HttpErrorResponse } from '@angular/common/http'
import { Button } from '@masmarino/gabarit/button'
import { EmptyState } from '@masmarino/gabarit/empty-state'
import { Spinner } from '@masmarino/gabarit/spinner'
import { PersonalRepositoryService } from '../application/personal-repository.service'
import { RepositoriesList } from '../repositories-list/repositories-list'
import { ConfirmModal } from '../../shared/confirm-modal/confirm-modal'
import { ToastService } from '../../shared/toast.service'
import { rejectionMessage } from '../../shared/api-error'
import { PageHeading } from '../../shared/page-heading/page-heading'

@Component({
  selector: 'app-my-repository-page',
  standalone: true,
  imports: [
    PageHeading,
    TranslocoPipe,
    Button,
    EmptyState,
    Spinner,
    RepositoriesList,
    ConfirmModal,
  ],
  templateUrl: './my-repository-page.html',
  styleUrl: './my-repository-page.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class MyRepositoryPage implements OnInit {
  private readonly personalRepositoryService = inject(PersonalRepositoryService)
  private readonly toastService = inject(ToastService)

  readonly loading = signal(true)
  readonly reserved = signal(false)
  readonly showConfirmModal = signal(false)
  readonly reserving = signal(false)

  ngOnInit(): void {
    this.personalRepositoryService.hasReservedNamespace().subscribe({
      next: (reserved) => {
        this.reserved.set(reserved)
        this.loading.set(false)
      },
      error: () => {
        // Assume not reserved: the create flow re-checks anyway.
        this.loading.set(false)
        this.toastService.error(t('repositories.mine.errors.checkFailed'))
      },
    })
  }

  openReserveModal(): void {
    this.showConfirmModal.set(true)
  }

  cancelReserve(): void {
    this.showConfirmModal.set(false)
  }

  confirmReserve(): void {
    if (this.reserving()) {
      return
    }
    this.reserving.set(true)
    this.personalRepositoryService.reserve().subscribe({
      next: () => {
        this.reserving.set(false)
        this.showConfirmModal.set(false)
        this.reserved.set(true)
      },
      error: (err) => {
        this.reserving.set(false)
        // 409: already reserved (by this tab or another); treat it as success.
        if (err instanceof HttpErrorResponse && err.status === 409) {
          this.showConfirmModal.set(false)
          this.reserved.set(true)
          return
        }
        this.toastService.error(rejectionMessage(err) ?? t('repositories.mine.errors.createFailed'))
      },
    })
  }
}
