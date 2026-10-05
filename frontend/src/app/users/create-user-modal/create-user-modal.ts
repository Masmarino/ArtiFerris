import { t } from '../../shared/i18n/translator'
import { TranslocoPipe } from '@jsverse/transloco'
import {
  ChangeDetectionStrategy,
  Component,
  ElementRef,
  Injector,
  afterNextRender,
  inject,
  output,
  signal,
  viewChild,
} from '@angular/core'
import { FormsModule } from '@angular/forms'
import { Alert } from '@masmarino/gabarit/alert'
import { Button } from '@masmarino/gabarit/button'
import { GbtInput } from '@masmarino/gabarit/input'
import { Modal } from '@masmarino/gabarit/modal'
import { Switch } from '@masmarino/gabarit/switch'
import { UsersService } from '../application/users.service'
import { InvitedUser } from '../domain/user.entity'
import { LinkMailFailed } from '../../shared/link-mail-failed/link-mail-failed'
import { errorCode } from '../../shared/api-error'

/** The server's own rule (`validate_email`), so the form refuses what it would refuse. */
function emailProblem(email: string): string | null {
  if (email === '') {
    return t('users.create.errors.emailRequired')
  }
  const [local, domain, ...rest] = email.split('@')
  const valid =
    rest.length === 0 &&
    !!local &&
    !!domain &&
    domain.includes('.') &&
    !domain.startsWith('.') &&
    !domain.endsWith('.') &&
    !/\s/.test(email)
  return valid ? null : t('users.create.errors.emailInvalid')
}

/**
 * The invitation dialog, as FerrisGit's: the address, the super-administrator switch, then the
 * outcome in the same dialog. Rendered under @if by the parent, so each opening starts fresh. A
 * refused invitation leaves the draft; an accepted one emits `invited` and shows the result.
 * `closed` is the parent's cue to remove it.
 */
@Component({
  selector: 'app-create-user-modal',
  standalone: true,
  imports: [TranslocoPipe, FormsModule, Alert, Button, GbtInput, LinkMailFailed, Modal, Switch],
  templateUrl: './create-user-modal.html',
  styleUrl: './create-user-modal.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class CreateUserModal {
  private readonly usersService = inject(UsersService)
  private readonly injector = inject(Injector)

  readonly invited = output<InvitedUser>()
  readonly closed = output<void>()

  readonly email = signal('')
  readonly isSuperAdmin = signal(false)
  readonly emailError = signal<string | null>(null)
  readonly formError = signal<string | null>(null)
  readonly sending = signal(false)
  /** The outcome once the account exists: sent, or the link to pass on when the mail did not go out. */
  readonly result = signal<{ email: string; user: InvitedUser } | null>(null)

  private readonly resultRegion = viewChild<ElementRef<HTMLElement>>('resultRegion')

  onEmailChange(value: string): void {
    this.email.set(value)
    this.emailError.set(null)
  }

  submit(): void {
    if (this.sending()) {
      return
    }
    const email = this.email().trim()
    const problem = emailProblem(email)
    this.emailError.set(problem)
    this.formError.set(null)
    if (problem) {
      return
    }
    this.sending.set(true)
    this.usersService.create(email, this.isSuperAdmin()).subscribe({
      next: (user) => {
        this.sending.set(false)
        this.result.set({ email, user })
        this.invited.emit(user)
        // The form that had the focus is gone: the focus moves to the result, the next thing to read.
        afterNextRender(() => this.resultRegion()?.nativeElement.focus(), {
          injector: this.injector,
        })
      },
      error: (error: unknown) => {
        this.sending.set(false)
        this.refused(error)
      },
    })
  }

  private refused(error: unknown): void {
    switch (errorCode(error)) {
      case 'email_taken':
        this.emailError.set(t('users.create.errors.emailTaken'))
        break
      case 'invalid_email':
        this.emailError.set(t('users.create.errors.emailInvalid'))
        break
      default:
        this.formError.set(t('users.create.errors.createFailed'))
    }
  }
}
