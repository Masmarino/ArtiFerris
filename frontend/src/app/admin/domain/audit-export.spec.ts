import { Subject, of } from 'rxjs'
import { AuditEntry, AuditPage } from './audit.entity'
import { AuditExportState, collectAuditPages, isPartialExport } from './audit-export'

const entry = (n: number): AuditEntry => ({
  aggregate_type: 'Admin',
  aggregate_id: `a${n}`,
  event_type: 'QuotaSet',
  payload: {},
  occurred_at: '2026-01-01T00:00:00Z',
  actor_id: null,
})
const entries = (from: number, count: number) =>
  Array.from({ length: count }, (_, i) => entry(from + i))

describe('collectAuditPages', () => {
  it('emits the loaded entries at once and stops when there is no cursor', () => {
    const fetchPage = vi.fn()
    const states: number[] = []

    collectAuditPages(entries(0, 3), null, fetchPage).subscribe((s) =>
      states.push(s.entries.length),
    )

    expect(states).toEqual([3])
    expect(fetchPage).not.toHaveBeenCalled()
  })

  it('follows the cursor to the last page, emitting after each one', () => {
    const pages: Record<string, AuditPage> = {
      c1: { entries: entries(3, 2), next_cursor: 'c2' },
      c2: { entries: entries(5, 1), next_cursor: null },
    }
    const fetchPage = vi.fn((cursor: string) => of(pages[cursor]))
    const states: { count: number; cursor: string | null }[] = []

    collectAuditPages(entries(0, 3), 'c1', fetchPage).subscribe((s) =>
      states.push({ count: s.entries.length, cursor: s.nextCursor }),
    )

    expect(states).toEqual([
      { count: 3, cursor: 'c1' },
      { count: 5, cursor: 'c2' },
      { count: 6, cursor: null },
    ])
    expect(fetchPage.mock.calls.map(([c]) => c)).toEqual(['c1', 'c2'])
  })

  it('stops at the row bound even though the server has more', () => {
    const fetchPage = vi.fn(() => of({ entries: entries(0, 4), next_cursor: 'more' }))
    let last: AuditExportState | undefined

    collectAuditPages(entries(0, 2), 'more', fetchPage, 10).subscribe((s) => (last = s))

    expect(last?.entries).toHaveLength(10)
    expect(last?.nextCursor).toBe('more')
    expect(isPartialExport(last!, 10)).toBe(true)
  })

  it('stops fetching when the subscriber unsubscribes', () => {
    const page$ = new Subject<AuditPage>()
    const fetchPage = vi.fn(() => page$)

    const subscription = collectAuditPages(entries(0, 1), 'c1', fetchPage).subscribe()
    subscription.unsubscribe()
    page$.next({ entries: entries(1, 1), next_cursor: 'c2' })

    expect(fetchPage).toHaveBeenCalledTimes(1)
  })

  it('reports a complete export as not partial', () => {
    expect(isPartialExport({ entries: entries(0, 5), nextCursor: null }, 10)).toBe(false)
  })
})
