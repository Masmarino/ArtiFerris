import { DestroyRef, Directive, ElementRef, afterNextRender, inject, signal } from '@angular/core'
import { ViewportObserver } from './viewport-observer'

/**
 * Adds `is-offscreen` while the element is outside the viewport, so looping animations can pause. It starts as "on
 * screen", which is what the prerendered page shows.
 */
@Directive({
  selector: '[appInView]',
  host: { '[class.is-offscreen]': 'offscreen()' },
})
export class InView {
  protected readonly offscreen = signal(false)

  constructor() {
    const element: HTMLElement = inject(ElementRef).nativeElement
    const viewport = inject(ViewportObserver)

    afterNextRender(() => viewport.observe(element, (visible) => this.offscreen.set(!visible)))
    inject(DestroyRef).onDestroy(() => viewport.unobserve(element))
  }
}
