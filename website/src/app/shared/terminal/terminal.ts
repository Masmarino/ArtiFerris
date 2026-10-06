import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core'

/**
 * The real output of a real command, as a terminal: a dark block in both themes, the lines printed one after the other,
 * then a prompt with a blinking cursor. The caption is projected and closes the block under a hairline, so the two
 * read as one exhibit. With `wrap`, long lines wrap with a hanging indent instead of scrolling.
 */
@Component({
  selector: 'app-terminal',
  template: `
    <figure class="proof on-dark">
      <pre
        class="proof__terminal"
        tabindex="0"
        [class.is-wrapping]="wrap()"
        [style.--lines]="lines().length"
        [attr.aria-label]="label()"
      ><code>@for (line of lines(); track $index) {<span class="proof__line">{{ line }}{{ $last ? '' : newline }}</span>}</code></pre>
      <figcaption><ng-content /></figcaption>
    </figure>
  `,
  styleUrl: './terminal.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class Terminal {
  readonly text = input.required<string>()
  /** Accessible name of the scrollable output, which says what it shows. */
  readonly label = input.required<string>()
  readonly wrap = input(false)

  /** One element per line, so a wrapped line hangs under its own start; the print animation takes one step each. */
  protected readonly lines = computed(() => this.text().split('\n'))
  protected readonly newline = '\n'
}
