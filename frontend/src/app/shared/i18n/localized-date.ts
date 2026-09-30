import { formatDate } from '@angular/common'
import { Pipe, PipeTransform } from '@angular/core'
import './locale-data'
import { activeLocale } from './translator'

type DateInput = string | number | Date | null | undefined

/** `formatDate` in the language the interface is displayed in. Empty for a missing date. */
export function formatLocalizedDate(value: DateInput, format = 'mediumDate'): string {
  return value === null || value === undefined || value === ''
    ? ''
    : formatDate(value, format, activeLocale())
}

/**
 * Stands in for Angular's `date` pipe, which is bound to the locale the application started in.
 * Same template syntax (`{{ value | date: 'short' }}`); a language change re-creates the views,
 * so a pure pipe is enough.
 */
@Pipe({ name: 'date' })
export class LocalizedDatePipe implements PipeTransform {
  transform(value: DateInput, format = 'mediumDate'): string | null {
    return value === null || value === undefined || value === ''
      ? null
      : formatLocalizedDate(value, format)
  }
}
