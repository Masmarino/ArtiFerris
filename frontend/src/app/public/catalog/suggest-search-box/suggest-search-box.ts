import { TranslocoPipe } from '@jsverse/transloco'
import {
  ChangeDetectionStrategy,
  Component,
  booleanAttribute,
  computed,
  effect,
  inject,
  input,
  model,
  output,
  signal,
} from '@angular/core'
import { takeUntilDestroyed } from '@angular/core/rxjs-interop'
import { FormControl, ReactiveFormsModule } from '@angular/forms'
import { Router } from '@angular/router'
import { GbtInput, type InputCombobox } from '@masmarino/gabarit/input'
import { Observable, Subject, catchError, of, switchMap, timer } from 'rxjs'
import { formatSuggestionsAnnouncement } from '../../../shared/format'
import { CatalogService } from '../application/catalog.service'
import { CatalogFormat, CatalogSuggestion, OwnerRef } from '../domain/catalog.entity'
import { packageLink } from '../domain/catalog-links'
import { CATALOGS } from '../domain/catalog.registry'
import { CATALOG_OVERLOAD } from '../domain/catalog-overload'
import { overloadMessage } from '../../../shared/api-error'

const DEBOUNCE_MS = 200
const MIN_LENGTH = 2
const MAX_LENGTH = 100
const MAX_SUGGESTIONS = 8

let nextId = 0

/**
 * A search input that suggests names as you type (ARIA combobox). Enter with nothing highlighted
 * emits `search`.
 */
@Component({
  imports: [TranslocoPipe, GbtInput, ReactiveFormsModule],
  selector: 'app-suggest-search-box',
  standalone: true,
  templateUrl: './suggest-search-box.html',
  styleUrl: './suggest-search-box.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class SuggestSearchBox {
  private readonly service = inject(CatalogService)
  private readonly router = inject(Router)
  private readonly typed = new Subject<string>()
  private dismissed = false
  // Pending: what is on screen no longer matches the latest keystroke.
  private stale = false

  readonly label = input.required<string>()
  readonly labelHidden = input(false, { transform: booleanAttribute })
  readonly placeholder = input('')
  readonly compact = input(false, { transform: booleanAttribute })
  readonly maxLength = input<number | null>(null)
  readonly format = input<CatalogFormat | null>(null)
  readonly owner = input<OwnerRef | null>(null)
  readonly value = model('')
  // eslint-disable-next-line @angular-eslint/no-output-native
  readonly search = output<string>()

  readonly inputId = `suggest-search-${nextId++}`
  readonly listboxId = `${this.inputId}-listbox`
  /** Gabarit's field, kept in step with `value` both ways. */
  protected readonly field = new FormControl('', { nonNullable: true })

  private readonly results = signal<CatalogSuggestion[] | null>(null)
  /** The server limited the last request, so an empty list is not "no match". */
  readonly notice = signal<string | null>(null)
  readonly open = signal(false)
  readonly activeIndex = signal(-1)

  readonly suggestions = computed(() => (this.results() ?? []).slice(0, MAX_SUGGESTIONS))
  readonly expanded = computed(() => this.open() && this.suggestions().length > 0)
  readonly noSuggestion = computed(
    () => this.open() && this.suggestions().length === 0 && this.notice() === null,
  )
  readonly activeDescendant = computed(() =>
    this.expanded() && this.activeIndex() >= 0 ? this.optionId(this.activeIndex()) : null,
  )
  readonly combobox = computed<InputCombobox>(() => ({
    expanded: this.expanded(),
    controls: this.listboxId,
    activeDescendant: this.activeDescendant(),
  }))
  readonly announcement = computed(() => {
    if (!this.open()) {
      return ''
    }
    return this.notice() ?? formatSuggestionsAnnouncement(this.suggestions().length)
  })

  constructor() {
    effect(() => {
      const value = this.value()
      if (value !== this.field.value) {
        this.field.setValue(value, { emitEvent: false })
      }
    })
    this.field.valueChanges.pipe(takeUntilDestroyed()).subscribe((text) => this.onInput(text))
    this.typed
      .pipe(
        switchMap((text) => this.fetch(text)),
        takeUntilDestroyed(),
      )
      .subscribe((results) => this.show(results))
  }

  optionId(index: number): string {
    return `${this.inputId}-option-${index}`
  }

  formatLabel(kind: CatalogFormat): string {
    return CATALOGS.find((catalog) => catalog.format === kind)?.label ?? kind
  }

  onInput(text: string): void {
    this.value.set(text)
    this.dismissed = false
    this.stale = true
    this.notice.set(null)
    this.activeIndex.set(-1)
    this.typed.next(text)
  }

  onKeydown(event: KeyboardEvent): void {
    switch (event.key) {
      case 'ArrowDown':
        event.preventDefault()
        this.move(1)
        break
      case 'ArrowUp':
        event.preventDefault()
        this.move(-1)
        break
      case 'Enter':
        if (event.isComposing) {
          return
        }
        event.preventDefault()
        if (this.activeDescendant() !== null) {
          this.select(this.activeIndex())
        } else {
          this.dismiss()
          this.search.emit(this.value())
        }
        break
      case 'Escape':
        if (this.open()) {
          event.preventDefault()
        }
        this.dismiss()
        break
    }
  }

  select(index: number): void {
    const suggestion = this.stale ? undefined : this.suggestions()[index]
    if (suggestion) {
      this.dismiss()
      void this.router.navigate(packageLink(suggestion))
    }
  }

  dismiss(): void {
    this.dismissed = true
    this.open.set(false)
    this.activeIndex.set(-1)
  }

  private move(step: 1 | -1): void {
    const count = this.stale ? 0 : this.suggestions().length
    if (count === 0) {
      return
    }
    if (!this.open()) {
      this.dismissed = false
      this.open.set(true)
      this.activeIndex.set(step === 1 ? 0 : count - 1)
      return
    }
    const current = this.activeIndex()
    this.activeIndex.set(
      step === 1 ? (current + 1) % count : current <= 0 ? count - 1 : current - 1,
    )
  }

  private fetch(raw: string): Observable<CatalogSuggestion[] | null> {
    const text = raw.trim().slice(0, MAX_LENGTH)
    if (text.length < MIN_LENGTH) {
      return of(null)
    }
    return timer(DEBOUNCE_MS).pipe(
      switchMap(() => this.service.suggest(text, { format: this.format(), owner: this.owner() })),
      catchError((error: unknown) => {
        this.notice.set(overloadMessage(error, CATALOG_OVERLOAD))
        return of(null)
      }),
    )
  }

  private show(results: CatalogSuggestion[] | null): void {
    this.stale = false
    this.results.set(results)
    this.activeIndex.set(-1)
    this.open.set((results !== null || this.notice() !== null) && !this.dismissed)
  }
}
