import { ChangeDetectionStrategy, Component, input } from '@angular/core'
import { TranslocoPipe } from '@jsverse/transloco'
import { Button } from '@masmarino/gabarit'
import { CodeBlock } from '../code-block/code-block'
import { APP_URL, DOCS } from '../links'
import { COMPOSE_COMMANDS } from '../snippets'

/**
 * What closes a page: where to try the product and where to install it, next to the commands that start it. The
 * installation page already opens on those commands, so it leaves them out (`commands`).
 */
@Component({
  selector: 'app-cta',
  imports: [TranslocoPipe, Button, CodeBlock],
  template: `
    <section class="section closing" aria-labelledby="cta-title">
      <div class="container">
        <div class="cta" [class.cta--commands]="commands()">
          <div class="cta__text">
            <h2 id="cta-title">{{ 'home.cta.title' | transloco }}</h2>
            <p class="lead">{{ 'home.cta.text' | transloco }}</p>
            <div class="actions">
              <a gbtButton variant="primary" size="large" [href]="appUrl">{{
                'nav.openApp' | transloco
              }}</a>
              <a gbtButton variant="secondary" size="large" [href]="docs.install" hreflang="fr">{{
                'home.cta.install' | transloco
              }}</a>
            </div>
          </div>
          @if (commands()) {
            <app-code-block
              class="cta__commands"
              language="bash"
              [code]="compose"
              [label]="'install.labels.compose' | transloco"
              [region]="'install.regions.compose' | transloco"
              [copyLabel]="'install.copy' | transloco"
              [copiedText]="'install.copied' | transloco"
              [failedText]="'install.copyFailed' | transloco"
            />
          }
        </div>
      </div>
    </section>
  `,
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class Cta {
  readonly commands = input(true)

  protected readonly appUrl = APP_URL
  protected readonly docs = DOCS
  protected readonly compose = COMPOSE_COMMANDS
}
