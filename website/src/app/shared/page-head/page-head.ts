import { ChangeDetectionStrategy, Component, input } from '@angular/core'
import { RouterLink } from '@angular/router'
import { GitField } from '../git-field/git-field'
import { injectActiveLang } from '../../i18n/active-lang'

/** The title block of an inner page: where it sits in the site, then whatever is projected (the h1 and a lead). */
@Component({
  selector: 'app-page-head',
  imports: [RouterLink, GitField],
  template: `
    <div class="page-band on-dark">
      <app-git-field variant="band" />
      <header class="page-head container">
        <p class="crumb">
          <a [routerLink]="['/', lang()]">ArtiFerris</a><span aria-hidden="true"> / </span
          ><span>{{ section() }}</span>
        </p>
        <ng-content />
      </header>
    </div>
  `,
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class PageHead {
  protected readonly lang = injectActiveLang()
  /** The translated name of the page, shown as the last step of the path. */
  readonly section = input.required<string>()
}
