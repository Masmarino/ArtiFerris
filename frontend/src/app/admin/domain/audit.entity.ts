export interface AuditEntry {
  aggregate_type: string
  aggregate_id: string
  event_type: string
  payload: unknown
  occurred_at: string
  actor_id: string | null
}

export interface AuditPage {
  entries: AuditEntry[]
  next_cursor: string | null
}

export interface AuditQuery {
  aggregate_type?: string
  exclude_aggregate_type?: string
  aggregate_id?: string
  actor_id?: string
  from?: string
  to?: string
  organization_id?: string
  cursor?: string
  limit?: number
}

export interface BlockedAccount {
  username: string
  remaining_seconds: number
}
