export interface MeResponse {
  id: string
  username: string
  is_super_admin: boolean
  is_organization_admin: boolean
  organization_id: string
  created_at: string
  language?: string | null
  /** The address ArtiFerris writes to, once verified; `null` otherwise (`undefined` from an older server). */
  email?: string | null
}
