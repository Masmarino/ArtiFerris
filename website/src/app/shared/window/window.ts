import { ChangeDetectionStrategy, Component, input } from '@angular/core'

/**
 * The product window around a demo or a screenshot. The title bar is decoration, hidden from assistive technology,
 * and the address is a plausible path, not a link.
 */
@Component({
  selector: 'app-window',
  template: `
    <div class="window__bar" aria-hidden="true">
      <span class="window__dots"><i></i><i></i><i></i></span>
      <span class="window__url"
        ><b>{{ url() }}</b></span
      >
      <span class="window__spacer"></span>
    </div>
    <div class="window__body"><ng-content /></div>
  `,
  styleUrl: './window.scss',
  host: { class: 'window' },
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class Window {
  readonly url = input('git.example.com')
}
