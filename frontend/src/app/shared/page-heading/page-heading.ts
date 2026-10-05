import { ChangeDetectionStrategy, Component, inject } from '@angular/core'
import { PageHeader } from '@masmarino/gabarit/page-header'
import { PageTitleService } from '../../shell/page-title.service'

/**
 * The page's own title, as FerrisGit's pages open: Gabarit's page header with the page's h1. The
 * shell's bar names only what is above the page, so every page under the shell starts with this. The
 * title is the one the shell keeps for the bar and the browser tab (the route's, or the one a detail
 * page sets once its data is in); until there is one, nothing is drawn. A line under the title goes
 * in `[header-meta]`, badges beside the title in `[header-badges]`, actions in `[header-actions]`.
 */
@Component({
  selector: 'app-page-heading',
  standalone: true,
  imports: [PageHeader],
  template: `
    @if (pageTitle.title(); as title) {
      <gbt-page-header [heading]="title">
        <ng-container ngProjectAs="[header-badges]"
          ><ng-content select="[header-badges]"
        /></ng-container>
        <ng-container ngProjectAs="[header-meta]"
          ><ng-content select="[header-meta]"
        /></ng-container>
        <ng-container ngProjectAs="[header-actions]"
          ><ng-content select="[header-actions]"
        /></ng-container>
      </gbt-page-header>
    }
  `,
  styles: ':host { display: block; }',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class PageHeading {
  protected readonly pageTitle = inject(PageTitleService)
}
