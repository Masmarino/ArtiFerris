export interface AdminApiToken {
  id: string
  user_id: string
  username: string
  label: string
  created_at: string
  last_used_at: string | null
  revoked_at: string | null
}

/** What the admin token list asks for, and the most the server ever returns in one page. */
export const ADMIN_TOKEN_PAGE_LIMIT = 500
