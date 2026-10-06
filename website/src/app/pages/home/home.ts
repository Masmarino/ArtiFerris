import { ChangeDetectionStrategy, Component } from '@angular/core'
import { RouterLink } from '@angular/router'
import { TranslocoPipe } from '@jsverse/transloco'
import { Button, CopyButton } from '@masmarino/gabarit'
import { injectActiveLang } from '../../i18n/active-lang'
import { usePageMeta } from '../../seo/page-meta'
import { Capabilities } from '../../shared/capabilities/capabilities'
import { Cta } from '../../shared/cta/cta'
import { GitField } from '../../shared/git-field/git-field'
import { InlineCodePipe } from '../../shared/inline-code.pipe'
import { APP_URL, GITHUB_URL, README_ROADMAP_URL } from '../../shared/links'
import { PlanFigure } from '../../shared/plan-figure/plan-figure'
import { HERO_COMMAND, PROOF_TERMINAL } from '../../shared/snippets'

import { Reveal } from '../../shared/motion/reveal.directive'
import { Terminal } from '../../shared/terminal/terminal'

@Component({
  selector: 'app-home',
  imports: [
    GitField,
    Reveal,
    RouterLink,
    TranslocoPipe,
    Button,
    CopyButton,
    Capabilities,
    InlineCodePipe,
    Cta,
    PlanFigure,
    Terminal,
  ],
  templateUrl: './home.html',
  styleUrl: './home.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class Home {
  protected readonly lang = injectActiveLang()

  protected readonly appUrl = APP_URL
  protected readonly githubUrl = GITHUB_URL
  protected readonly readmeRoadmapUrl = README_ROADMAP_URL
  protected readonly heroCommand = HERO_COMMAND
  protected readonly proofTerminal = PROOF_TERMINAL

  // Texts are under home.why.<id>, home.limits.<id>, home.made.crates.<id> and home.roadmap.<id>.
  protected readonly why = ['proxy', 'tenants', 'scan', 'access'] as const
  protected readonly limits = ['formats', 'storage', 'saml', 'provenance'] as const
  protected readonly crates = [
    'domain',
    'application',
    'infrastructure',
    'api',
    'npm',
    'docker',
  ] as const
  // The roadmap of the README has no versions: its items are shown by subject.
  protected readonly nextItems = [
    'formats',
    'storage',
    'ha',
    'provenance',
    'scopes',
    'saml',
  ] as const

  constructor() {
    usePageMeta('home')
  }
}
