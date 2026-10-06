import { Pipe, PipeTransform, inject } from '@angular/core'
import { DomSanitizer, SafeHtml } from '@angular/platform-browser'

const ESCAPES: Record<string, string> = {
  '&': '&amp;',
  '<': '&lt;',
  '>': '&gt;',
  '"': '&quot;',
  "'": '&#39;',
}

/** Escapes the text, then turns `backtick` spans into <code>, so translations stay plain strings. */
export function inlineCodeToHtml(text: string): string {
  const escaped = text.replace(/[&<>"']/g, (char) => ESCAPES[char])
  return escaped.replace(/`([^`]+)`/g, '<code>$1</code>')
}

@Pipe({ name: 'inlineCode' })
export class InlineCodePipe implements PipeTransform {
  private readonly sanitizer = inject(DomSanitizer)

  // Safe: everything was escaped before <code>, the only tag, was added.
  transform(text: string): SafeHtml {
    return this.sanitizer.bypassSecurityTrustHtml(inlineCodeToHtml(text))
  }
}
