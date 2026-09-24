import { EMPTY, Observable, expand, map, of } from 'rxjs'
import { AuditEntry, AuditPage } from './audit.entity'

/** The most rows a CSV export will pull; past it the file is cut and the UI says so. */
export const AUDIT_EXPORT_MAX_ROWS = 10_000

export interface AuditExportState {
  entries: AuditEntry[]
  /** Set while the server still has older entries. */
  nextCursor: string | null
}

/**
 * Starts from what is already loaded and follows the cursor, emitting after each page.
 * Stops when the log is exhausted or `maxRows` is reached; unsubscribing cancels the export.
 */
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

/** True when the export stopped before the end of the log. */
export function isPartialExport(state: AuditExportState, maxRows = AUDIT_EXPORT_MAX_ROWS): boolean {
  return state.nextCursor !== null || state.entries.length > maxRows
}
