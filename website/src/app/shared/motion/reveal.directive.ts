import {
  DestroyRef,
  Directive,
  ElementRef,
  afterNextRender,
  computed,
  inject,
  input,
  signal,
} from '@angular/core'
import { MotionService } from './motion.service'
import { ViewportObserver } from './viewport-observer'

/**
 * Fades an element in the first time it scrolls into view. `[appReveal]="index"` staggers siblings: the index becomes
 * the CSS variable --i, which the stylesheet turns into a delay.
 *
 * The prerendered HTML only has the data-reveal attribute. The hidden state lives in the stylesheet under html.js, so
 * without JavaScript the element is simply visible.
 */
@Directive({
  selector: '[appReveal]',
  host: {
    '[attr.data-reveal]': "mode() === 'fade' ? '' : null",
    '[class.is-revealed]': 'revealed()',
    '[style.--i]': 'order()',
  },
})
export class Reveal {
  readonly index = input<number | string>(0, { alias: 'appReveal' })
  /**
   * `fade` (default) hides the element until it's seen, then fades it in. `mark` hides nothing and only adds
   * `is-revealed`: the roadmap timeline uses it to light up a step as it scrolls into view.
   */
  readonly mode = input<'fade' | 'mark'>('fade', { alias: 'appRevealMode' })

  protected readonly order = computed(() => Number(this.index()) || 0)
  protected readonly revealed = signal(false)

  constructor() {
    const element: HTMLElement = inject(ElementRef).nativeElement
    const motion = inject(MotionService)
    const viewport = inject(ViewportObserver)

    afterNextRender(() => {
      if (motion.reducedMotion() || !viewport.supported) {
        this.revealed.set(true)
        return
      }
      viewport.observeOnce(element, () => this.revealed.set(true))
    })
    inject(DestroyRef).onDestroy(() => viewport.unobserve(element))
  }
}
