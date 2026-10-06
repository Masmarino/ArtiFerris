import { ChangeDetectionStrategy, Component } from '@angular/core'
import { TranslocoPipe } from '@jsverse/transloco'
import { usePageMeta } from '../../seo/page-meta'
import { Cta } from '../../shared/cta/cta'
import { Install } from '../../shared/install/install'
import { PageHead } from '../../shared/page-head/page-head'

@Component({
  selector: 'app-install-page',
  imports: [TranslocoPipe, Cta, Install, PageHead],
  templateUrl: './install.html',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class InstallPage {
  constructor() {
    usePageMeta('install')
  }
}
