export interface MfaStatus {
  totp_enabled: boolean
  backup_codes_remaining: number
  passkey_count: number
}

export interface TotpEnrollment {
  secret: string
  otpauth_url: string
}

export interface BackupCodes {
  backup_codes: string[]
}

export interface PasskeySummary {
  id: string
  name: string
  created_at: string
}

export interface PasskeyRegistrationStart {
  challenge_id: string
  // Passed to createPasskeyCredential as is.
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  public_key: any
}
