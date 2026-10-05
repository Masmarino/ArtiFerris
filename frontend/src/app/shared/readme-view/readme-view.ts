import { TranslocoPipe } from '@jsverse/transloco'
import {
  ChangeDetectionStrategy,
  Component,
  ElementRef,
  ViewEncapsulation,
  afterRenderEffect,
  computed,
  input,
  viewChild,
} from '@angular/core'
import { Card } from '@masmarino/gabarit/card'
import { EmptyState } from '@masmarino/gabarit/empty-state'
import { t } from '../i18n/translator'

@Component({
  selector: 'app-readme-view',
  standalone: true,
  imports: [TranslocoPipe, Card, EmptyState],
  templateUrl: './readme-view.html',
  styleUrl: './readme-view.scss',
  // The markup is created at runtime, so emulated encapsulation would never match it.
  encapsulation: ViewEncapsulation.None,
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ReadmeView {
  readonly html = input<string | null>(null)

  readonly content = computed(() => this.html()?.trim() || null)

  private readonly body = viewChild<ElementRef<HTMLElement>>('body')

  constructor() {
    afterRenderEffect({
      write: () => {
        this.content()
        const body = this.body()?.nativeElement
        if (body) {
          makeScrollableBlocksFocusable(body, {
            codeBlock: t('shared.readme.codeBlock'),
            table: t('shared.readme.table'),
          })
        }
      },
    })
  }
}

/**
 * A long code block or a wide table scrolls sideways, which a keyboard can only do once it takes focus (WCAG 2.1.1):
 * each one becomes a named stop in the tab order. A table keeps its own role and gets only the stop and the name.
 */
export function makeScrollableBlocksFocusable(
  container: HTMLElement,
  labels: { codeBlock: string; table: string },
): void {
  for (const pre of Array.from(container.querySelectorAll('pre'))) {
    pre.setAttribute('tabindex', '0')
    pre.setAttribute('role', 'region')
    pre.setAttribute('aria-label', labels.codeBlock)
  }
  for (const table of Array.from(container.querySelectorAll('table'))) {
    table.setAttribute('tabindex', '0')
    table.setAttribute('aria-label', labels.table)
  }
}
