import { ChangeDetectionStrategy, Component, afterNextRender, inject, signal } from '@angular/core'
import { DOCUMENT } from '@angular/common'
import { TranslocoPipe } from '@jsverse/transloco'
import { CodeBlock, CodeLanguage } from '../code-block/code-block'
import { InlineCodePipe } from '../inline-code.pipe'
import { DOCS } from '../links'
import { COMPOSE_COMMANDS, HELM_COMMANDS, SOURCE_COMMANDS } from '../snippets'
import { tabIndexForKey } from '../tabs-keyboard'

interface InstallTab {
  id: 'compose' | 'helm' | 'source'
  code: string
  language: CodeLanguage
  doc: string
}

/**
 * The three ways to run ArtiFerris as tabs, with the commands from the docs and a copy button on each. Without scripts
 * the three blocks follow one another; the ARIA tab roles are only added once scripts run.
 */
import { Reveal } from '../motion/reveal.directive'

@Component({
  selector: 'app-install',
  imports: [Reveal, TranslocoPipe, CodeBlock, InlineCodePipe],
  templateUrl: './install.html',
  styleUrl: './install.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class Install {
  private readonly document = inject(DOCUMENT)

  protected readonly tabs: InstallTab[] = [
    { id: 'compose', code: COMPOSE_COMMANDS, language: 'bash', doc: DOCS.install },
    { id: 'helm', code: HELM_COMMANDS, language: 'bash', doc: DOCS.install },
    { id: 'source', code: SOURCE_COMMANDS, language: 'bash', doc: DOCS.install },
  ]
  protected readonly steps = ['secrets', 'start', 'signin'] as const
  protected readonly needs = ['docker', 'postgres', 'storage', 'https'] as const

  protected readonly active = signal(0)
  protected readonly enhanced = signal(false)

  constructor() {
    afterNextRender(() => this.enhanced.set(true))
  }

  protected select(index: number, focus = false): void {
    this.active.set(index)
    if (focus) this.document.getElementById(`install-tab-${this.tabs[index].id}`)?.focus()
  }

  protected onKeydown(event: KeyboardEvent, index: number): void {
    const next = tabIndexForKey(event.key, index, this.tabs.length)
    if (next === null) return
    event.preventDefault()
    this.select(next, true)
  }
}
