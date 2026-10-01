const SEPARATOR = ';'
const PLAIN_NUMBER = /^-?\d+(?:\.\d+)?$/

function escapeField(value: unknown): string {
  const raw = value === null || value === undefined ? '' : String(value)
  const lines = raw.replace(/\r\n?/g, '\n')
  // A formula trigger at the start of any line can start a record in a lenient parser.
  const safe = PLAIN_NUMBER.test(lines) ? lines : lines.replace(/(^|\n)(?=[=+\-@\t])/g, "$1'")
  return /[";\r\n]/.test(safe) ? `"${safe.replace(/"/g, '""')}"` : safe
}

export function toCsv<T>(rows: T[], columns: { key: string; label: string }[]): string {
  const header = columns.map((c) => escapeField(c.label)).join(SEPARATOR)
  const lines = rows.map((row) =>
    columns.map((c) => escapeField((row as Record<string, unknown>)[c.key])).join(SEPARATOR),
  )
  return [header, ...lines].join('\r\n')
}

/** With a BOM and `;`, French Excel opens the file with accents and columns intact. */
export function csvBlob(csv: string): Blob {
  return new Blob(['﻿', csv], { type: 'text/csv;charset=utf-8' })
}
