export interface LoginResponse {
  token: string | null
  mfa_token: string | null
  mfa_setup_required: boolean
  mfa_has_totp: boolean
  mfa_has_passkey: boolean
}

export interface LoginOutcome {
  mfaRequired: boolean
  mfaToken?: string
  mfaSetupRequired?: boolean
  mfaHasTotp?: boolean
  mfaHasPasskey?: boolean
}

export interface TotpSetupEnrollment {
  secret: string
  otpauth_url: string
}

export interface TotpSetupComplete {
  token: string
  backup_codes: string[]
}

export interface SsoConfig {
  type: 'ldap' | 'oidc' | null
  registration_enabled: boolean
}
