import { t } from '../../shared/i18n/translator'
import { TranslocoPipe } from '@jsverse/transloco'
import {
  ChangeDetectionStrategy,
  Component,
  computed,
  effect,
  inject,
  input,
  output,
  signal,
} from '@angular/core'
import { FormsModule } from '@angular/forms'
import { Button } from '@masmarino/gabarit/button'
import { Modal } from '@masmarino/gabarit/modal'
import { Select } from '@masmarino/gabarit/select'
import { ConfirmService } from '../../shared/confirm.service'
import { ROLE_OPTIONS, Role } from '../domain/permission.entity'

@Component({
  selector: 'app-permission-role-editor',
  standalone: true,
  imports: [TranslocoPipe, Modal, Button, Select, FormsModule],
  templateUrl: './permission-role-editor.html',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class PermissionRoleEditor {
  private readonly confirmService = inject(ConfirmService)

  readonly isOpen = input.required<boolean>()
  readonly label = input.required<string>()
  readonly currentRole = input.required<Role>()
  readonly subjectKind = input<'user' | 'repository'>('user')
  readonly saving = input(false)

  readonly roleChanged = output<Role>()
  readonly revoked = output<void>()
  readonly closed = output<void>()

  readonly selectedRole = signal<Role>('read')
  readonly roleOptions = ROLE_OPTIONS

  readonly modalTitle = computed(() =>
    this.subjectKind() === 'repository'
      ? t('repositories.permissions.titleRepository', { label: this.label() })
      : t('repositories.permissions.titleUser', { label: this.label() }),
  )

  constructor() {
    // The modal is reused across rows: re-seed the role on open.
    effect(() => {
      if (this.isOpen()) {
        this.selectedRole.set(this.currentRole())
      }
    })
  }

  onRoleSelect(role: string): void {
    this.selectedRole.set(role as Role)
  }

  save(): void {
    this.roleChanged.emit(this.selectedRole())
  }

  async revoke(): Promise<void> {
    const label = this.label()
    const message =
      this.subjectKind() === 'repository'
        ? t('repositories.permissions.revokeRepository', { label })
        : t('repositories.permissions.revokeUser', { label })
    const confirmed = await this.confirmService.ask({
      heading: t('repositories.permissions.revokeHeading'),
      message,
      confirmLabel: t('repositories.permissions.revoke'),
      danger: true,
    })
    if (!confirmed) {
      return
    }
    this.revoked.emit()
  }
}
