import { Component, inject } from '@angular/core'
import { toSignal } from '@angular/core/rxjs-interop'
import { Router } from '@angular/router'
import { map } from 'rxjs'

/** For stories: prints the router's current URL, so a play function can see where a navigation ended up. */
@Component({
  selector: 'app-story-current-url',
  standalone: true,
  template: '<output aria-label="URL courante">{{ url() }}</output>',
})
export class CurrentUrl {
  private readonly router = inject(Router)
  readonly url = toSignal(this.router.events.pipe(map(() => this.router.url)), {
    initialValue: this.router.url,
  })
}
