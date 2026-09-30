import { TranslocoPipe } from '@jsverse/transloco'
import { ChangeDetectionStrategy, Component, inject } from '@angular/core'
import { Button, ConfirmDangerModal, Modal } from '@masmarino/gabarit'
import { ConfirmService } from '../confirm.service'

@Component({
  selector: 'app-confirm-host',
  standalone: true,
  imports: [TranslocoPipe, Modal, Button, ConfirmDangerModal],
  templateUrl: './confirm-host.html',
  styleUrl: './confirm-host.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ConfirmHost {
  protected readonly confirm = inject(ConfirmService)
}
