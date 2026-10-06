import { ChangeDetectionStrategy, Component, inject } from '@angular/core'
import { TranslocoPipe } from '@jsverse/transloco'
import { Icon } from '@masmarino/gabarit'
import { ThemeStore } from '../../theme/theme-store'

@Component({
  selector: 'app-theme-toggle',
  imports: [TranslocoPipe, Icon],
  templateUrl: './theme-toggle.html',
  styleUrl: './theme-toggle.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ThemeToggle {
  protected readonly theme = inject(ThemeStore)
}
