import { Directive, Injectable, inject, input, signal } from '@angular/core'

/**
 * The circle number on the architecture plan that the visitor points at, or the tile or section it leads to.
 * Each side lights up when the other is pointed at.
 */
@Injectable({ providedIn: 'root' })
export class PlanHighlight {
  readonly active = signal<number | null>(null)

  set(number: number | null): void {
    this.active.set(number)
  }
}

/** Ties an element to a circle of the plan: pointing at it or focusing inside it lights the circle. */
@Directive({
  selector: '[appPlanPoint]',
  host: {
    '(mouseenter)': 'highlight.set(circle())',
    '(mouseleave)': 'highlight.set(null)',
    '(focusin)': 'highlight.set(circle())',
    '(focusout)': 'highlight.set(null)',
  },
})
export class PlanPoint {
  protected readonly highlight = inject(PlanHighlight)
  readonly circle = input.required<number>({ alias: 'appPlanPoint' })
}
