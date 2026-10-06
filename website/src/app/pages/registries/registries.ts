import { ChangeDetectionStrategy, Component } from '@angular/core'
import { TranslocoPipe } from '@jsverse/transloco'
import { usePageMeta } from '../../seo/page-meta'
import { CodeBlock } from '../../shared/code-block/code-block'
import { Cta } from '../../shared/cta/cta'
import { GroupSim } from '../../shared/group-sim/group-sim'
import { InlineCodePipe } from '../../shared/inline-code.pipe'
import { DOCS } from '../../shared/links'
import { PageHead } from '../../shared/page-head/page-head'
import { DOCKER_EXAMPLE } from '../../shared/snippets'

import { Reveal } from '../../shared/motion/reveal.directive'

@Component({
  selector: 'app-registries',
  imports: [Reveal, TranslocoPipe, Cta, CodeBlock, GroupSim, InlineCodePipe, PageHead],
  templateUrl: './registries.html',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class Registries {
  protected readonly docs = DOCS
  protected readonly dockerCommands = DOCKER_EXAMPLE

  // Texts are under home.registries.<id>.
  protected readonly facts = ['types', 'quota', 'retention', 'audit', 'scan', 'public'] as const

  constructor() {
    usePageMeta('registries')
  }
}
