import { ChangeDetectionStrategy, Component, computed, inject } from '@angular/core'
import { RouterLink } from '@angular/router'
import { TranslocoPipe } from '@jsverse/transloco'
import { injectActiveLang } from '../../i18n/active-lang'
import { ArchitecturePlan } from '../architecture-plan/architecture-plan'
import { Reveal } from '../motion/reveal.directive'
import { PlanHighlight, PlanPoint } from '../plan-highlight'
import { PLAN_FRAGMENTS, planPageCommands } from '../plan-links'

/**
 * The architecture plan as a figure: grid paper, a caption, a key and the seven numbers as a list of links, which is
 * how keyboard users and small screens reach what the circles point at. The plan draws itself when it first scrolls
 * into view.
 */
@Component({
  selector: 'app-plan-figure',
  imports: [RouterLink, TranslocoPipe, ArchitecturePlan, PlanPoint, Reveal],
  templateUrl: './plan-figure.html',
  styleUrl: './plan-figure.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class PlanFigure {
  private readonly lang = injectActiveLang()
  protected readonly plan = inject(PlanHighlight)

  protected readonly numbers = [1, 2, 3, 4, 5, 6, 7]
  protected readonly commands = computed(() => planPageCommands(this.lang()))

  protected fragment(n: number): string {
    return PLAN_FRAGMENTS[n - 1]
  }
}
