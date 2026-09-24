import { Pipe, PipeTransform } from '@angular/core'
import { formatBytes } from './format'

/** Pure pipe: memoized by Angular per input value, unlike calling formatBytes() directly in a template. */
@Pipe({ name: 'formatBytes' })
export class FormatBytesPipe implements PipeTransform {
  /** A missing size shows as a dash. */
  transform(bytes: number | null | undefined): string {
    return bytes === null || bytes === undefined ? '—' : formatBytes(bytes)
  }
}
