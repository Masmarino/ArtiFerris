export interface ApiToken {
  id: string
  label: string
  created_at: string
  last_used_at: string | null
}

export const API_TOKEN_LIFETIME_DAYS = { session: 7, reauthenticated: 365 } as const

export interface CreatedApiToken {
  id: string
  token: string
}
