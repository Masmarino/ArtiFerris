import { t } from '../i18n/translator'
import { TranslocoPipe } from '@jsverse/transloco'
import {
  ChangeDetectionStrategy,
  Component,
  ElementRef,
  booleanAttribute,
  computed,
  input,
  output,
  viewChild,
} from '@angular/core'
import { Alert } from '@masmarino/gabarit/alert'
import { CopyField } from '@masmarino/gabarit/copy-field'
import { InvitationMail, PasswordResetMail } from '../invitation-mail'

export type MailedLinkKind = 'invitation' | 'password-reset'

/**
 * A link whose mail did not go out, an invitation's or a password reset's, as FerrisGit shows it: why, then the link
 * to pass on, since the administrator sees it this once.
 */
@Component({
  selector: 'app-link-mail-failed',
  standalone: true,
  imports: [TranslocoPipe, Alert, CopyField],
  templateUrl: './link-mail-failed.html',
  styleUrl: './link-mail-failed.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class LinkMailFailed {
  /** Who the link is for: their address while the account is pending. */
  readonly name = input.required<string>()
  readonly mail = input.required<InvitationMail | PasswordResetMail>()
  readonly kind = input<MailedLinkKind>('invitation')
  readonly dismissible = input(false, { transform: booleanAttribute })

  readonly dismissed = output<void>()

  protected readonly reason = computed(() => {
    switch (this.mail().email_error) {
      case 'email_not_configured':
        return t('invitationMail.notConfigured')
      case 'email_send_failed':
        return t('invitationMail.sendFailed')
      case 'email_no_address':
        return t('invitationMail.noAddress')
      default:
        return null
    }
  })

  protected readonly link = computed(() => {
    const mail = this.mail()
    return 'reset_url' in mail
      ? mail.reset_url
      : 'activation_url' in mail
        ? mail.activation_url
        : undefined
  })

  /** What the alert says about the link: how long it lasts and which action issues another. */
  protected readonly wording = computed(() =>
    this.kind() === 'password-reset'
      ? {
          help: 'invitationMail.resetHelp',
          copy: 'invitationMail.resetCopy',
          noLink: 'invitationMail.resetNoLink',
        }
      : {
          help: 'invitationMail.help',
          copy: 'invitationMail.copy',
          noLink: 'invitationMail.noLink',
        },
  )

  private readonly region = viewChild<ElementRef<HTMLElement>>('region')

  /** The alert shows up away from what was clicked, so the page moves the focus to it. */
  focus(): void {
    this.region()?.nativeElement.focus()
  }
}
