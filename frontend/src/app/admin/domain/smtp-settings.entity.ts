export type SmtpSecurity = 'none' | 'start_tls' | 'tls'

export interface SmtpSettings {
  host: string
  port: number
  username: string
  from_name: string
  from_address: string
  security: SmtpSecurity
  password_set: boolean
}

export interface UnreadableSmtpSettings {
  secret_unreadable: true
  error: string
}

export type SmtpSettingsResponse = SmtpSettings | UnreadableSmtpSettings

export function isUnreadableSmtpSettings(
  settings: SmtpSettingsResponse,
): settings is UnreadableSmtpSettings {
  return 'secret_unreadable' in settings && settings.secret_unreadable
}

export interface UpdateSmtpSettings {
  host: string
  port: number
  username: string
  /** Omitted keeps the stored password. */
  password?: string
  from_name: string
  from_address: string
  security: SmtpSecurity
}
