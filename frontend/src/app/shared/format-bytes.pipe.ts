import { Pipe, PipeTransform } from '@angular/core'
import { formatBytes } from './format'

@Pipe({ name: 'formatBytes' })
export class FormatBytesPipe implements PipeTransform {
  transform(bytes: number | null | undefined): string {
    return bytes === null || bytes === undefined ? '—' : formatBytes(bytes)
  }
}
