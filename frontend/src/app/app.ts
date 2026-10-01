import {
  ChangeDetectionStrategy,
  Component,
  effect,
  inject,
  signal,
  untracked,
  viewChild,
} from '@angular/core'
import { RouterOutlet } from '@angular/router'
import { LanguageService } from './shared/i18n/language.service'

@Component({
  selector: 'app-root',
  imports: [RouterOutlet],
  templateUrl: './app.html',
  styleUrl: './app.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class App {
  private readonly languageService = inject(LanguageService)
  private readonly outlet = viewChild(RouterOutlet)

  protected readonly viewGeneration = signal(0)

  constructor() {
    let firstRun = true
    effect(() => {
      this.languageService.language()
      untracked(() => {
        if (
          firstRun ||
          this.outlet()?.activatedRouteData['recreatesViewsOnLanguageChange'] === true
        ) {
          firstRun = false
          return
        }
        this.viewGeneration.update((generation) => generation + 1)
      })
    })
  }
}
