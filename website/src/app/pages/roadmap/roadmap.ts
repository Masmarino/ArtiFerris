import { ChangeDetectionStrategy, Component } from '@angular/core'
import { TranslocoPipe } from '@jsverse/transloco'
import { Button } from '@masmarino/gabarit'
import { usePageMeta } from '../../seo/page-meta'
import { InlineCodePipe } from '../../shared/inline-code.pipe'
import { CHANGELOG_URL, FERRISGIT_URL, README_ROADMAP_URL } from '../../shared/links'
import { PageHead } from '../../shared/page-head/page-head'

interface RoadmapGroup {
  id: string
  /** The version, shown as a mono mark in front of the title. Groups without one are not scheduled. */
  tag?: string
  items: number
  hasIntro: boolean
  /** "Planned" for a scheduled version or list, "Under consideration" for ideas with no version. */
  status: 'planned' | 'considering'
}

// Mirrors the Feuille de route section of the README, which has no versions; the texts are under roadmap.groups.<id>.
const GROUPS: RoadmapGroup[] = [
  { id: 'formats', items: 1, hasIntro: true, status: 'planned' },
  { id: 'scale', items: 3, hasIntro: true, status: 'planned' },
  { id: 'trust', items: 3, hasIntro: false, status: 'planned' },
]

import { Reveal } from '../../shared/motion/reveal.directive'

@Component({
  selector: 'app-roadmap',
  imports: [Reveal, TranslocoPipe, Button, InlineCodePipe, PageHead],
  templateUrl: './roadmap.html',
  styleUrl: './roadmap.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class Roadmap {
  protected readonly groups = GROUPS.map((group) => ({
    ...group,
    numbers: Array.from({ length: group.items }, (_, index) => index + 1),
  }))
  protected readonly changelogUrl = CHANGELOG_URL
  protected readonly readmeUrl = README_ROADMAP_URL
  protected readonly ferrisgitUrl = FERRISGIT_URL

  constructor() {
    usePageMeta('roadmap')
  }
}
