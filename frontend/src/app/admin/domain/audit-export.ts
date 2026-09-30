import { EMPTY, Observable, expand, map, of } from 'rxjs'
import { AuditEntry, AuditPage } from './audit.entity'

export const AUDIT_EXPORT_MAX_ROWS = 10_000

export interface AuditExportState {
  entries: AuditEntry[]
  nextCursor: string | null
}

export function collectAuditPages(
  loaded: AuditEntry[],
  cursor: string | null,
  fetchPage: (cursor: string) => Observable<AuditPage>,
  maxRows = AUDIT_EXPORT_MAX_ROWS,
): Observable<AuditExportState> {
  return of<AuditExportState>({ entries: loaded, nextCursor: cursor }).pipe(
    expand((state) =>
      state.nextCursor !== null && state.entries.length < maxRows
        ? fetchPage(state.nextCursor).pipe(
            map((page) => ({
              entries: [...state.entries, ...page.entries],
              nextCursor: page.next_cursor,
            })),
          )
        : EMPTY,
    ),
  )
}

export function isPartialExport(state: AuditExportState, maxRows = AUDIT_EXPORT_MAX_ROWS): boolean {
  return state.nextCursor !== null || state.entries.length > maxRows
}
