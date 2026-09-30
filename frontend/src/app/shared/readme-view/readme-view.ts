import { TranslocoPipe } from '@jsverse/transloco'
import {
  ChangeDetectionStrategy,
  Component,
  ViewEncapsulation,
  computed,
  input,
} from '@angular/core'
import { Card, EmptyState } from '@masmarino/gabarit'

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
}
