import { HttpErrorResponse } from '@angular/common/http'
import { Component } from '@angular/core'
import { ComponentFixture, TestBed } from '@angular/core/testing'
import { Router, provideRouter } from '@angular/router'
import { Observable, Subject, of, throwError } from 'rxjs'
import { CatalogService } from '../application/catalog.service'
import {
  CatalogFormat,
  CatalogSuggestion,
  OwnerRef,
  SuggestOptions,
} from '../domain/catalog.entity'
import { catalogSuggestion, dockerSuggestion } from '../testing/catalog.fixtures'
import { SuggestSearchBox } from './suggest-search-box'

const SUGGESTIONS = [
  catalogSuggestion({ name: 'left-pad' }),
  catalogSuggestion({
    name: 'left-pad-extra',
    owner: { kind: 'personal', slug: 'bob', display_name: 'Bob' },
  }),
  dockerSuggestion(),
]

interface Options {
  suggest?: (text: string, options?: SuggestOptions) => Observable<CatalogSuggestion[]>
  format?: CatalogFormat | null
  owner?: OwnerRef | null
  value?: string
}

function render(options: Options = {}) {
  vi.useFakeTimers()
  const suggest = vi.fn(options.suggest ?? (() => of(SUGGESTIONS)))
  TestBed.configureTestingModule({
    providers: [provideRouter([]), { provide: CatalogService, useValue: { suggest } }],
  })
  const navigate = vi.spyOn(TestBed.inject(Router), 'navigate').mockResolvedValue(true)
  const fixture = TestBed.createComponent(SuggestSearchBox)
  fixture.componentRef.setInput('label', 'Rechercher un paquet')
  if (options.format !== undefined) {
    fixture.componentRef.setInput('format', options.format)
  }
  if (options.owner !== undefined) {
    fixture.componentRef.setInput('owner', options.owner)
  }
  if (options.value !== undefined) {
    fixture.componentRef.setInput('value', options.value)
  }
  const search = vi.fn()
  fixture.componentInstance.search.subscribe(search)
  fixture.detectChanges()
  const el = fixture.nativeElement as HTMLElement
  return { fixture, el, input: el.querySelector('input')!, suggest, navigate, search }
}

function type(fixture: ComponentFixture<unknown>, input: HTMLInputElement, text: string) {
  input.value = text
  input.dispatchEvent(new Event('input'))
  fixture.detectChanges()
}

function wait(fixture: ComponentFixture<unknown>, ms = 200) {
  vi.advanceTimersByTime(ms)
  fixture.detectChanges()
}

function press(
  fixture: ComponentFixture<unknown>,
  input: HTMLInputElement,
  key: string,
  init: KeyboardEventInit = {},
) {
  const event = new KeyboardEvent('keydown', { key, cancelable: true, bubbles: true, ...init })
  input.dispatchEvent(event)
  fixture.detectChanges()
  return event
}

function typeAndWait(fixture: ComponentFixture<unknown>, input: HTMLInputElement, text = 'left') {
  type(fixture, input, text)
  wait(fixture)
}

function options(el: HTMLElement): HTMLElement[] {
  return Array.from(el.querySelectorAll<HTMLElement>('[role="option"]'))
}

function announcement(el: HTMLElement): string {
  return el.querySelector('[aria-live="polite"]')!.textContent!
}

describe('SuggestSearchBox', () => {
  afterEach(() => vi.useRealTimers())

  describe('markup', () => {
    it('is a combobox controlling a listbox, collapsed at first', () => {
      const { el, input } = render()

      const listbox = el.querySelector('[role="listbox"]')!
      expect(input.getAttribute('role')).toBe('combobox')
      expect(input.getAttribute('aria-autocomplete')).toBe('list')
      expect(input.getAttribute('aria-expanded')).toBe('false')
      expect(input.getAttribute('aria-controls')).toBe(listbox.id)
      expect(input.hasAttribute('aria-activedescendant')).toBe(false)
      expect(input.getAttribute('autocomplete')).toBe('off')
    })

    it('labels the input, visibly by default', () => {
      const { el, input } = render()

      const label = el.querySelector<HTMLLabelElement>(`label[for="${input.id}"]`)!
      expect(label.textContent).toBe('Rechercher un paquet')
      expect(label.classList).not.toContain('gbt-input__label--hidden')
    })

    it('can hide the label visually while keeping it for screen readers', () => {
      const { fixture, el, input } = render()

      fixture.componentRef.setInput('labelHidden', true)
      fixture.detectChanges()

      expect(el.querySelector(`label[for="${input.id}"]`)!.classList).toContain(
        'gbt-input__label--hidden',
      )
    })

    it('shows the placeholder and the initial value', () => {
      const { fixture, input } = render({ value: 'demo' })
      fixture.componentRef.setInput('placeholder', 'Nom du paquet')
      fixture.detectChanges()

      expect(input.value).toBe('demo')
      expect(input.placeholder).toBe('Nom du paquet')
    })

    it('follows the value set from outside', () => {
      const { fixture, input } = render({ value: 'demo' })

      fixture.componentRef.setInput('value', 'other')
      fixture.detectChanges()

      expect(input.value).toBe('other')
    })

    it('limits the length only when asked to', () => {
      const { fixture, input } = render()
      expect(input.hasAttribute('maxlength')).toBe(false)

      fixture.componentRef.setInput('maxLength', 100)
      fixture.detectChanges()

      expect(input.getAttribute('maxlength')).toBe('100')
    })

    it('gives two boxes different ids', () => {
      const first = render()
      const second = TestBed.createComponent(SuggestSearchBox)
      second.componentRef.setInput('label', 'Autre')
      second.detectChanges()

      const other = second.nativeElement.querySelector('input') as HTMLInputElement
      expect(other.id).not.toBe(first.input.id)
      expect(other.getAttribute('aria-controls')).not.toBe(
        first.input.getAttribute('aria-controls'),
      )
    })

    it('reports typing through valueChange', () => {
      const { fixture, input } = render()
      const changes: string[] = []
      fixture.componentInstance.value.subscribe((value) => changes.push(value))

      type(fixture, input, 'le')

      expect(changes).toEqual(['le'])
    })
  })

  describe('fetching', () => {
    it('does not ask for suggestions below 2 characters', () => {
      const { fixture, input, suggest } = render()

      typeAndWait(fixture, input, 'l')
      typeAndWait(fixture, input, ' l ')

      expect(suggest).not.toHaveBeenCalled()
    })

    it('waits 200 ms after the last keystroke, then asks once with the trimmed text', () => {
      const { fixture, input, suggest } = render()

      type(fixture, input, 'le')
      wait(fixture, 150)
      type(fixture, input, ' lef ')
      wait(fixture, 199)
      expect(suggest).not.toHaveBeenCalled()

      wait(fixture, 1)
      expect(suggest).toHaveBeenCalledTimes(1)
      expect(suggest).toHaveBeenCalledWith('lef', { format: null, owner: null })
    })

    it('caps the text at 100 characters', () => {
      const { fixture, input, suggest } = render()

      typeAndWait(fixture, input, 'x'.repeat(150))

      expect(suggest).toHaveBeenCalledWith('x'.repeat(100), { format: null, owner: null })
    })

    it('never lets a slow old response overwrite a newer one', () => {
      const answers = new Map<string, Subject<CatalogSuggestion[]>>()
      const { fixture, el, input, suggest } = render({
        suggest: (text) => {
          const subject = new Subject<CatalogSuggestion[]>()
          answers.set(text, subject)
          return subject
        },
      })

      typeAndWait(fixture, input, 'le')
      const old = answers.get('le')!
      typeAndWait(fixture, input, 'left')
      expect(old.observed).toBe(false)

      answers.get('left')!.next([catalogSuggestion({ name: 'left-pad' })])
      old.next([catalogSuggestion({ name: 'old-answer' })])
      fixture.detectChanges()

      expect(suggest).toHaveBeenCalledTimes(2)
      expect(options(el).map((option) => option.textContent)).toEqual([
        expect.stringContaining('left-pad'),
      ])
    })

    it('does not let the previous suggestions be picked while the new answer is pending', () => {
      const pending = new Subject<CatalogSuggestion[]>()
      let call = 0
      const { fixture, el, input, navigate } = render({
        suggest: () => (call++ === 0 ? of(SUGGESTIONS) : pending),
      })
      typeAndWait(fixture, input, 'le')
      expect(options(el)).toHaveLength(3)

      typeAndWait(fixture, input, 'zzz')
      press(fixture, input, 'ArrowDown')
      press(fixture, input, 'Enter')
      options(el)[0].click()

      expect(navigate).not.toHaveBeenCalled()
      expect(input.hasAttribute('aria-activedescendant')).toBe(false)
    })

    it('makes the new suggestions pickable once they arrive', () => {
      const pending = new Subject<CatalogSuggestion[]>()
      let call = 0
      const { fixture, el, input, navigate } = render({
        suggest: () => (call++ === 0 ? of(SUGGESTIONS) : pending),
      })
      typeAndWait(fixture, input, 'le')
      typeAndWait(fixture, input, 'left')

      pending.next([catalogSuggestion({ name: 'left-pad' })])
      fixture.detectChanges()
      options(el)[0].click()

      expect(navigate).toHaveBeenCalledTimes(1)
    })

    it('drops a pending request as soon as the user types again', () => {
      const pending = new Subject<CatalogSuggestion[]>()
      const { fixture, input } = render({ suggest: () => pending })

      typeAndWait(fixture, input, 'le')
      expect(pending.observed).toBe(true)

      type(fixture, input, 'lef')

      expect(pending.observed).toBe(false)
    })

    it('closes the popup and drops the request when the text falls below 2 characters', () => {
      const { fixture, el, input } = render()
      typeAndWait(fixture, input)
      expect(options(el)).toHaveLength(3)

      type(fixture, input, 'l')

      expect(options(el)).toHaveLength(0)
      expect(input.getAttribute('aria-expanded')).toBe('false')
      expect(announcement(el)).toBe('')
    })

    it('swallows a failed request without any error message', () => {
      const { fixture, el, input } = render({
        suggest: () => throwError(() => new HttpErrorResponse({ status: 500 })),
      })

      typeAndWait(fixture, input)

      expect(input.getAttribute('aria-expanded')).toBe('false')
      expect(el.querySelector('[role="alert"]')).toBeNull()
      expect(announcement(el)).toBe('')
    })

    it.each([
      [503, 'Le catalogue est momentanément occupé'],
      [429, 'Trop de requêtes, patientez un instant puis réessayez.'],
    ])(
      'tells the visitor when the server answers a %i, and clears it once typing resumes',
      (status, message) => {
        let attempt = 0
        const { fixture, el, input } = render({
          suggest: () =>
            ++attempt === 1 ? throwError(() => new HttpErrorResponse({ status })) : of(SUGGESTIONS),
        })

        typeAndWait(fixture, input, 'le')

        expect(el.querySelector('.suggest__empty')!.textContent).toBe(message)
        expect(el.querySelector('.suggest__empty')!.textContent).not.toContain('Aucune suggestion')
        expect(announcement(el)).toBe(message)

        typeAndWait(fixture, input, 'left')

        expect(el.querySelector('.suggest__empty')).toBeNull()
        expect(options(el)).toHaveLength(3)
      },
    )

    it('keeps suggesting after the server answered 429', () => {
      let attempt = 0
      const { fixture, el, input } = render({
        suggest: () =>
          ++attempt === 1
            ? throwError(() => new HttpErrorResponse({ status: 429 }))
            : of(SUGGESTIONS),
      })
      typeAndWait(fixture, input, 'le')
      expect(options(el)).toHaveLength(0)

      typeAndWait(fixture, input, 'left')

      expect(options(el)).toHaveLength(3)
    })
  })

  describe('suggestions', () => {
    it('shows a format badge, the name and the owner and repository of each', () => {
      const { fixture, el, input } = render()

      typeAndWait(fixture, input)

      const [npm, , docker] = options(el)
      expect(npm.querySelector('.suggest__format')!.textContent).toBe('npm')
      expect(npm.querySelector('.suggest__name')!.textContent).toBe('left-pad')
      expect(npm.querySelector('.suggest__meta')!.textContent).toBe('par admin / test-npm')
      expect(docker.querySelector('.suggest__format')!.textContent).toBe('Docker')
      expect(docker.querySelector('.suggest__meta')!.textContent).toBe('par Acme Corp / images')
    })

    it('expands the combobox and gives every option a unique id', () => {
      const { fixture, el, input } = render()

      typeAndWait(fixture, input)

      const ids = options(el).map((option) => option.id)
      expect(input.getAttribute('aria-expanded')).toBe('true')
      expect(new Set(ids).size).toBe(3)
      expect(ids.every((id) => id.length > 0)).toBe(true)
    })

    it('shows at most 8', () => {
      const many = Array.from({ length: 12 }, (_, index) =>
        catalogSuggestion({ name: `pkg-${index}` }),
      )
      const { fixture, el, input } = render({ suggest: () => of(many) })

      typeAndWait(fixture, input)

      expect(options(el)).toHaveLength(8)
      expect(announcement(el)).toBe('8 suggestions disponibles')
    })

    it('says so when nothing matches', () => {
      const { fixture, el, input } = render({ suggest: () => of([]) })

      typeAndWait(fixture, input, 'zzz')

      expect(input.getAttribute('aria-expanded')).toBe('false')
      expect(el.querySelector('.suggest__empty')!.textContent).toContain('Aucune suggestion')
      expect(announcement(el)).toBe('Aucune suggestion')
    })

    it('announces the count, singular or plural', () => {
      const one = render({ suggest: () => of([SUGGESTIONS[0]]) })
      typeAndWait(one.fixture, one.input)
      expect(announcement(one.el)).toBe('1 suggestion disponible')
      TestBed.resetTestingModule()

      const three = render()
      typeAndWait(three.fixture, three.input)
      expect(announcement(three.el)).toBe('3 suggestions disponibles')
    })

    it('asks the server for the locked format and owner instead of filtering a global list', () => {
      const format = render({ format: 'docker' })
      typeAndWait(format.fixture, format.input)
      expect(format.suggest).toHaveBeenCalledWith('left', { format: 'docker', owner: null })
      TestBed.resetTestingModule()

      const owner = render({ owner: { kind: 'organization', slug: 'acme' } })
      typeAndWait(owner.fixture, owner.input)
      expect(owner.suggest).toHaveBeenCalledWith('left', {
        format: null,
        owner: { kind: 'organization', slug: 'acme' },
      })
    })

    it('sends both filters together', () => {
      const { fixture, input, suggest } = render({
        format: 'npm',
        owner: { kind: 'personal', slug: 'admin' },
      })

      typeAndWait(fixture, input)

      expect(suggest).toHaveBeenCalledWith('left', {
        format: 'npm',
        owner: { kind: 'personal', slug: 'admin' },
      })
    })

    it('shows what the server answered for the filters, without filtering again', () => {
      const { fixture, el, input } = render({
        format: 'docker',
        suggest: () => of([SUGGESTIONS[0]]),
      })

      typeAndWait(fixture, input)

      expect(options(el)).toHaveLength(1)
      expect(announcement(el)).toBe('1 suggestion disponible')
    })
  })

  describe('keyboard', () => {
    function opened() {
      const rendered = render()
      typeAndWait(rendered.fixture, rendered.input)
      return rendered
    }

    it('highlights nothing until an arrow key is pressed', () => {
      const { el, input } = opened()

      expect(input.hasAttribute('aria-activedescendant')).toBe(false)
      expect(options(el).every((option) => option.getAttribute('aria-selected') === 'false')).toBe(
        true,
      )
    })

    it('moves the highlight down and reflects it in aria-activedescendant', () => {
      const { fixture, el, input } = opened()

      const event = press(fixture, input, 'ArrowDown')
      expect(event.defaultPrevented).toBe(true)
      expect(input.getAttribute('aria-activedescendant')).toBe(options(el)[0].id)
      expect(options(el)[0].getAttribute('aria-selected')).toBe('true')

      press(fixture, input, 'ArrowDown')
      expect(input.getAttribute('aria-activedescendant')).toBe(options(el)[1].id)
      expect(options(el)[0].getAttribute('aria-selected')).toBe('false')
      expect(options(el)[1].getAttribute('aria-selected')).toBe('true')
    })

    it('wraps around from the last option to the first going down', () => {
      const { fixture, el, input } = opened()

      for (let i = 0; i < 4; i++) {
        press(fixture, input, 'ArrowDown')
      }

      expect(input.getAttribute('aria-activedescendant')).toBe(options(el)[0].id)
    })

    it('starts from the last option going up, and wraps to it from the first', () => {
      const { fixture, el, input } = opened()

      press(fixture, input, 'ArrowUp')
      expect(input.getAttribute('aria-activedescendant')).toBe(options(el)[2].id)

      press(fixture, input, 'ArrowUp')
      press(fixture, input, 'ArrowUp')
      expect(input.getAttribute('aria-activedescendant')).toBe(options(el)[0].id)

      press(fixture, input, 'ArrowUp')
      expect(input.getAttribute('aria-activedescendant')).toBe(options(el)[2].id)
    })

    it('forgets the highlight when the user types again', () => {
      const { fixture, input } = opened()
      press(fixture, input, 'ArrowDown')

      type(fixture, input, 'lefty')

      expect(input.hasAttribute('aria-activedescendant')).toBe(false)
    })

    it('leaves Home and End to the text field', () => {
      const { fixture, input } = opened()
      press(fixture, input, 'ArrowDown')

      expect(press(fixture, input, 'Home').defaultPrevented).toBe(false)
      expect(press(fixture, input, 'End').defaultPrevented).toBe(false)
      expect(input.hasAttribute('aria-activedescendant')).toBe(true)
    })

    it('opens the highlighted suggestion on Enter', () => {
      const { fixture, input, navigate, search } = opened()
      press(fixture, input, 'ArrowDown')
      press(fixture, input, 'ArrowDown')

      const event = press(fixture, input, 'Enter')

      expect(event.defaultPrevented).toBe(true)
      expect(navigate).toHaveBeenCalledWith([
        '/@bob',
        'test-npm',
        'packages',
        'npm',
        'left-pad-extra',
      ])
      expect(search).not.toHaveBeenCalled()
      expect(input.getAttribute('aria-expanded')).toBe('false')
    })

    it('sends an organization suggestion under /o/:slug', () => {
      const { fixture, input, navigate } = opened()
      press(fixture, input, 'ArrowUp')

      press(fixture, input, 'Enter')

      expect(navigate).toHaveBeenCalledWith([
        '/o',
        'acme',
        'images',
        'packages',
        'docker',
        'team/api',
      ])
    })

    it('emits search with the text on Enter when nothing is highlighted', () => {
      const { fixture, input, navigate, search } = opened()

      const event = press(fixture, input, 'Enter')

      expect(event.defaultPrevented).toBe(true)
      expect(search).toHaveBeenCalledExactlyOnceWith('left')
      expect(navigate).not.toHaveBeenCalled()
      expect(input.getAttribute('aria-expanded')).toBe('false')
    })

    it('emits search on Enter even when no suggestion popup is open', () => {
      const { fixture, input, search } = render()
      type(fixture, input, 'a')

      press(fixture, input, 'Enter')

      expect(search).toHaveBeenCalledExactlyOnceWith('a')
    })

    it('emits the text untouched, for the host to trim', () => {
      const { fixture, input, search } = render()
      type(fixture, input, '  ')

      press(fixture, input, 'Enter')

      expect(search).toHaveBeenCalledWith('  ')
    })

    it('ignores Enter while an input method is composing', () => {
      const { fixture, input, search } = opened()

      const event = press(fixture, input, 'Enter', { isComposing: true })

      expect(event.defaultPrevented).toBe(false)
      expect(search).not.toHaveBeenCalled()
    })

    it('does not show suggestions that arrive after Enter', () => {
      const pending = new Subject<CatalogSuggestion[]>()
      const { fixture, input } = render({ suggest: () => pending })
      typeAndWait(fixture, input)

      press(fixture, input, 'Enter')
      pending.next(SUGGESTIONS)
      fixture.detectChanges()

      expect(input.getAttribute('aria-expanded')).toBe('false')
    })

    it('closes on Escape and keeps the text', () => {
      const { fixture, el, input } = opened()
      press(fixture, input, 'ArrowDown')

      const event = press(fixture, input, 'Escape')

      expect(event.defaultPrevented).toBe(true)
      expect(input.getAttribute('aria-expanded')).toBe('false')
      expect(input.hasAttribute('aria-activedescendant')).toBe(false)
      expect(input.value).toBe('left')
      expect(announcement(el)).toBe('')
    })

    it('leaves a second Escape alone', () => {
      const { fixture, input } = opened()
      press(fixture, input, 'Escape')

      const event = press(fixture, input, 'Escape')

      expect(event.defaultPrevented).toBe(false)
      expect(input.value).toBe('left')
    })

    it('does not reopen for a response that arrives after Escape', () => {
      const pending = new Subject<CatalogSuggestion[]>()
      const { fixture, input } = render({ suggest: () => pending })
      typeAndWait(fixture, input)

      press(fixture, input, 'Escape')
      pending.next(SUGGESTIONS)
      fixture.detectChanges()

      expect(input.getAttribute('aria-expanded')).toBe('false')
    })

    it('reopens on ArrowDown after Escape, with the first option highlighted', () => {
      const { fixture, el, input } = opened()
      press(fixture, input, 'Escape')

      press(fixture, input, 'ArrowDown')

      expect(input.getAttribute('aria-expanded')).toBe('true')
      expect(input.getAttribute('aria-activedescendant')).toBe(options(el)[0].id)
    })

    it('reopens on ArrowUp after Escape, with the last option highlighted', () => {
      const { fixture, el, input } = opened()
      press(fixture, input, 'Escape')

      press(fixture, input, 'ArrowUp')

      expect(input.getAttribute('aria-activedescendant')).toBe(options(el)[2].id)
    })

    it('does nothing on arrow keys when there is no suggestion', () => {
      const { fixture, input } = render({ suggest: () => of([]) })
      typeAndWait(fixture, input, 'zzz')

      press(fixture, input, 'ArrowDown')

      expect(input.getAttribute('aria-expanded')).toBe('false')
      expect(input.hasAttribute('aria-activedescendant')).toBe(false)
    })
  })

  describe('pointer', () => {
    it('opens the clicked suggestion', () => {
      const { fixture, el, input, navigate } = render()
      typeAndWait(fixture, input)

      options(el)[2].click()
      fixture.detectChanges()

      expect(navigate).toHaveBeenCalledWith([
        '/o',
        'acme',
        'images',
        'packages',
        'docker',
        'team/api',
      ])
      expect(input.getAttribute('aria-expanded')).toBe('false')
    })

    it('keeps the focus on the input when the popup is pressed', () => {
      const { fixture, el, input } = render()
      typeAndWait(fixture, input)

      const event = new MouseEvent('mousedown', { bubbles: true, cancelable: true })
      options(el)[0].dispatchEvent(event)

      expect(event.defaultPrevented).toBe(true)
    })

    it('closes when the input loses focus', () => {
      const { fixture, input } = render()
      typeAndWait(fixture, input)

      input.dispatchEvent(new Event('blur'))
      fixture.detectChanges()

      expect(input.getAttribute('aria-expanded')).toBe('false')
    })

    it('does not open for a response that arrives after the input lost focus', () => {
      const pending = new Subject<CatalogSuggestion[]>()
      const { fixture, input } = render({ suggest: () => pending })
      typeAndWait(fixture, input)

      input.dispatchEvent(new Event('blur'))
      pending.next(SUGGESTIONS)
      fixture.detectChanges()

      expect(input.getAttribute('aria-expanded')).toBe('false')
    })
  })
})

@Component({
  standalone: true,
  imports: [SuggestSearchBox],
  template: '<app-suggest-search-box label="Chercher" [value]="text" (search)="found = $event" />',
})
class Host {
  text = 'demo'
  found = ''
}

describe('SuggestSearchBox in a host', () => {
  afterEach(() => vi.useRealTimers())

  it('hands the host the text to search for', () => {
    TestBed.configureTestingModule({
      providers: [
        provideRouter([]),
        { provide: CatalogService, useValue: { suggest: () => of([]) } },
      ],
    })
    const fixture = TestBed.createComponent(Host)
    fixture.detectChanges()
    const input: HTMLInputElement = fixture.nativeElement.querySelector('input')
    expect(input.value).toBe('demo')

    input.value = 'left'
    input.dispatchEvent(new Event('input'))
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))

    expect(fixture.componentInstance.found).toBe('left')
  })
})
