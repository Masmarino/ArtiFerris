import { ChangeDetectionStrategy, Component, computed, inject, input } from '@angular/core'
import { DomSanitizer, SafeHtml } from '@angular/platform-browser'
import { CopyButton } from '@masmarino/gabarit'
import hljs from 'highlight.js/lib/core'
import bash from 'highlight.js/lib/languages/bash'
import yaml from 'highlight.js/lib/languages/yaml'

// Only the two languages the site shows, to keep the highlighter small.
hljs.registerLanguage('bash', bash)
hljs.registerLanguage('yaml', yaml)

export type CodeLanguage = 'bash' | 'yaml'

/**
 * A highlighted snippet with a title bar and an optional copy button. The HTML comes from highlight.js, which escapes
 * whatever it doesn't wrap, and the snippets are constants of the site, so bypassing the sanitizer is safe here.
 * Every line is wrapped so it can be marked (`markedLines`). Shell commands wrap at the edge with a hanging indent, so a
 * long line is read whole instead of cut; YAML keeps its lines, where the indentation is the meaning.
 */
@Component({
  selector: 'app-code-block',
  imports: [CopyButton],
  templateUrl: './code-block.html',
  styleUrl: './code-block.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { '[class.is-printing]': 'print()', '[class.is-wrapping]': "language() === 'bash'" },
})
export class CodeBlock {
  private readonly sanitizer = inject(DomSanitizer)

  readonly code = input.required<string>()
  readonly language = input.required<CodeLanguage>()
  /** Shown above the code: a file name, or what the commands do. */
  readonly label = input.required<string>()
  /** Accessible name of the scrollable area, which describes the content, not the control. */
  readonly region = input.required<string>()
  readonly copyLabel = input<string | null>(null)
  readonly copiedText = input('')
  readonly failedText = input('')
  /** 1-based numbers of the lines to mark, for example the job a demo makes fail. */
  readonly markedLines = input<readonly number[]>([])
  /** The lines come in one after the other, like output being printed, each time the block is shown. */
  readonly print = input(false)

  protected readonly html = computed<SafeHtml>(() => {
    const marked = new Set(this.markedLines())
    // highlight.js never opens a span across a line break for these two languages.
    const lines = hljs
      .highlight(this.code().replace(/\n$/, ''), { language: this.language() })
      .value.split('\n')
      .map(
        (line, index) =>
          `<span class="code__line${marked.has(index + 1) ? ' is-marked' : ''}">${line}\n</span>`,
      )
    return this.sanitizer.bypassSecurityTrustHtml(lines.join(''))
  })
}
