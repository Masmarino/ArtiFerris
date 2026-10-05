/** Why a mail carrying a link did not go out, as the server names it. */
export type MailError = 'email_not_configured' | 'email_send_failed' | 'email_no_address'

/**
 * What became of an invitation's mail, as the server reports it with the invited account or after a resend. When the
 * mail could not go out, `activation_url` is the link to pass on: the only copy, shown this once.
 */
export interface InvitationMail {
  email_sent: boolean
  email_error?: MailError
  activation_url?: string
}

/** The same for a password reset, whose link is `reset_url`. */
export interface PasswordResetMail {
  email_sent: boolean
  email_error?: MailError
  reset_url?: string
}
