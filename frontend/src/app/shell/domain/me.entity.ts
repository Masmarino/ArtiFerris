export interface MeResponse {
  id: string
  username: string
  is_super_admin: boolean
  is_organization_admin: boolean
  organization_id: string
  created_at: string
  /** The language the user chose: `null` until they have, absent from a server that predates the setting. */
  language?: string | null
}
