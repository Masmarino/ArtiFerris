export interface ApiToken {
  id: string
  label: string
  created_at: string
  last_used_at: string | null
}

/** A token created from a session alone is short-lived; confirming the password gets the long lifetime. */
export const API_TOKEN_LIFETIME_DAYS = { session: 7, reauthenticated: 365 } as const

export interface CreatedApiToken {
  id: string
  token: string
}
