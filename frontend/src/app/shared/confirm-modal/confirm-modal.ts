import { t } from '../i18n/translator'
import { TranslocoPipe } from '@jsverse/transloco'
import { ChangeDetectionStrategy, Component, input, output } from '@angular/core'
import { Button } from '@masmarino/gabarit/button'
import { Modal } from '@masmarino/gabarit/modal'

@Component({
  selector: 'app-confirm-modal',
  standalone: true,
  imports: [TranslocoPipe, Modal, Button],
  templateUrl: './confirm-modal.html',
  styleUrl: './confirm-modal.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ConfirmModal {
  readonly heading = input.required<string>()
  readonly message = input.required<string>()
  readonly confirmLabel = input(t('common.confirm'))
  readonly confirming = input(false)

  readonly confirmed = output<void>()
  readonly cancelled = output<void>()
}
