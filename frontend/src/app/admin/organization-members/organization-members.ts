import { t } from '../../shared/i18n/translator'
import { TranslocoPipe } from '@jsverse/transloco'
import { ChangeDetectionStrategy, Component, effect, inject, input, signal } from '@angular/core'
import { FormsModule } from '@angular/forms'
import { Button, Checkbox, EmptyState, GbtInput, Spinner, Tooltip } from '@masmarino/gabarit'
import { OrganizationMembersService } from '../application/organization-members.service'
import { OrganizationMember } from '../domain/organization-member.entity'
import { ConfirmService } from '../../shared/confirm.service'
import { ToastService } from '../../shared/toast.service'

@Component({
  selector: 'app-organization-members',
  standalone: true,
  imports: [TranslocoPipe, Button, EmptyState, GbtInput, Checkbox, FormsModule, Spinner, Tooltip],
  templateUrl: './organization-members.html',
  styleUrl: './organization-members.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class OrganizationMembers {
  private readonly organizationMembersService = inject(OrganizationMembersService)
  private readonly confirmService = inject(ConfirmService)
  private readonly toastService = inject(ToastService)

  readonly organizationId = input.required<string>()

  readonly members = signal<OrganizationMember[]>([])
  readonly loading = signal(true)
  readonly errorMessage = signal<string | null>(null)

  readonly addingMember = signal(false)
  readonly newUsername = signal('')
  readonly newEmail = signal('')
  readonly newIsOrganizationAdmin = signal(false)
  readonly inviting = signal(false)

  // effect(), not ngOnInit — this component is reused across organizations on the same route.
  constructor() {
    effect(() => {
      this.organizationId()
      this.members.set([])
      this.addingMember.set(false)
      this.newUsername.set('')
      this.newEmail.set('')
      this.newIsOrganizationAdmin.set(false)
      this.inviting.set(false)
      this.loading.set(true)
      this.reload()
    })
  }

  private reload(): void {
    const organizationId = this.organizationId()
    const stillCurrent = () => this.organizationId() === organizationId
    this.errorMessage.set(null)
    this.organizationMembersService.list(organizationId).subscribe({
      next: (members) => {
        if (stillCurrent()) {
          this.members.set(members)
          this.loading.set(false)
        }
      },
      error: () => {
        if (stillCurrent()) {
          this.loading.set(false)
          this.errorMessage.set(t('admin.members.errors.loadFailed'))
        }
      },
    })
  }

  startAdding(): void {
    this.addingMember.set(true)
    this.newUsername.set('')
    this.newEmail.set('')
    this.newIsOrganizationAdmin.set(false)
  }

  cancelAdding(): void {
    this.addingMember.set(false)
  }

  invite(): void {
    if (this.newUsername().trim() === '' || this.newEmail().trim() === '' || this.inviting()) {
      return
    }
    this.inviting.set(true)
    const organizationId = this.organizationId()
    const stillCurrent = () => this.organizationId() === organizationId
    const username = this.newUsername()
    this.organizationMembersService
      .invite(organizationId, username, this.newEmail(), this.newIsOrganizationAdmin())
      .subscribe({
        next: () => {
          if (stillCurrent()) {
            this.inviting.set(false)
            this.addingMember.set(false)
            this.reload()
          }
          this.toastService.success(t('admin.members.invited', { username }))
        },
        error: () => {
          if (stillCurrent()) {
            this.inviting.set(false)
          }
          this.toastService.error(t('admin.members.errors.inviteFailed'))
        },
      })
  }

  async toggleOrganizationAdmin(member: OrganizationMember): Promise<void> {
    const promoting = !member.is_organization_admin
    const confirmed = await this.confirmService.ask({
      heading: promoting ? t('admin.members.promoteHeading') : t('admin.members.demoteHeading'),
      message: t(promoting ? 'admin.members.promoteConfirm' : 'admin.members.demoteConfirm', {
        username: member.username,
      }),
      confirmLabel: promoting ? t('admin.members.promote') : t('admin.members.demote'),
      danger: !promoting,
    })
    if (!confirmed) return
    this.organizationMembersService
      .setOrganizationAdmin(this.organizationId(), member.id, promoting)
      .subscribe({
        next: () => {
          this.reload()
          this.toastService.success(
            promoting
              ? t('admin.members.promoted', { username: member.username })
              : t('admin.members.demoted', { username: member.username }),
          )
        },
        error: () => this.toastService.error(t('admin.members.errors.updateFailed')),
      })
  }
}
