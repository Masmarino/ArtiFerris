import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  afterNextRender,
  inject,
  signal,
} from '@angular/core'
import { DOCUMENT } from '@angular/common'
import { takeUntilDestroyed } from '@angular/core/rxjs-interop'
import { ActivatedRoute, RouterLink } from '@angular/router'
import { TranslocoPipe } from '@jsverse/transloco'
import { injectActiveLang } from '../../i18n/active-lang'
import { usePageMeta } from '../../seo/page-meta'
import { Cta } from '../../shared/cta/cta'
import { InlineCodePipe } from '../../shared/inline-code.pipe'
import { PageHead } from '../../shared/page-head/page-head'
import { ScreenName, Screenshot } from './screenshot/screenshot'
import { tabIndexForKey } from '../../shared/tabs-keyboard'
import { Window } from '../../shared/window/window'

// Section headings are anchors named feature-<id>; a link to one opens its tab.
const ANCHOR_PREFIX = 'feature-'

interface FeatureSection {
  id: string
  bullets: number
  screen?: ScreenName
  /** The address shown in the window bar of the screenshot. */
  url?: string
}

import { Reveal } from '../../shared/motion/reveal.directive'

@Component({
  selector: 'app-features',
  imports: [Reveal, RouterLink, TranslocoPipe, Cta, InlineCodePipe, PageHead, Screenshot, Window],
  templateUrl: './features.html',
  styleUrl: './features.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class Features {
  private readonly document = inject(DOCUMENT)

  protected readonly lang = injectActiveLang()

  // Domains with a screenshot are tabs; without JavaScript the bar is hidden and the panels follow one another.
  // Texts are under features.sections.<id> and features.tabs.items.<id>.
  protected readonly tabs: FeatureSection[] = [
    { id: 'repos', bullets: 5, screen: 'repositories', url: 'acme.artiferris.pro/repositories' },
    {
      id: 'packages',
      bullets: 5,
      screen: 'package',
      url: 'acme.artiferris.pro/repositories/npm-hosted/packages/npm/left-pad',
    },
    {
      id: 'scan',
      bullets: 3,
      screen: 'scan',
      url: 'acme.artiferris.pro/repositories/images/packages/docker/api',
    },
    {
      id: 'tenants',
      bullets: 4,
      screen: 'organizations',
      url: 'app.artiferris.pro/admin/organizations',
    },
    { id: 'public', bullets: 3, screen: 'explore', url: 'artiferris.pro/explorer' },
    { id: 'docs', bullets: 3, screen: 'docs', url: 'app.artiferris.pro/docs/utilisation/npm' },
  ]

  // Tiles below the tabs, of different sizes: what matters most gets the most room.
  protected readonly more: FeatureSection[] = [
    { id: 'accounts', bullets: 4 },
    { id: 'admin', bullets: 4 },
    { id: 'languages', bullets: 2 },
    { id: 'tech', bullets: 4 },
  ]
  protected readonly missingItems = [1, 2, 3, 4, 5, 6]

  /** Index of the visible panel; the prerendered page shows the first. */
  protected readonly active = signal(0)
  /** The tab roles are only added once scripts run; until then the panels are plain sections. */
  protected readonly enhanced = signal(false)

  constructor() {
    usePageMeta('features')

    afterNextRender(() => {
      this.enhanced.set(true)
      this.openTabOf(this.document.defaultView?.location.hash.replace('#', ''))
    })
    // A link to a fragment of this page opens the matching tab.
    inject(ActivatedRoute)
      .fragment.pipe(takeUntilDestroyed(inject(DestroyRef)))
      .subscribe((fragment) => this.openTabOf(fragment))
  }

  /** `/en/features/#feature-scan` opens the analysis tab. Other fragments are left to the browser. */
  private openTabOf(fragment: string | null | undefined): void {
    const target = fragment?.replace(ANCHOR_PREFIX, '')
    const index = this.tabs.findIndex((tab) => tab.id === target)
    if (index >= 0 && index !== this.active()) {
      this.active.set(index)
    }
  }

  protected bulletKeys(section: FeatureSection): number[] {
    return Array.from({ length: section.bullets }, (_, index) => index + 1)
  }

  protected select(index: number, focus = false): void {
    if (index === this.active()) return
    this.active.set(index)
    if (focus) this.document.getElementById(`tab-${this.tabs[index].id}`)?.focus()
  }

  protected onKeydown(event: KeyboardEvent, index: number): void {
    const next = tabIndexForKey(event.key, index, this.tabs.length)
    if (next === null) return
    event.preventDefault()
    this.select(next, true)
  }
}
