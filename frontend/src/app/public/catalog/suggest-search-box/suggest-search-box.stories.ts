import {
  applicationConfig,
  moduleMetadata,
  type Decorator,
  type Meta,
  type StoryObj,
} from '@storybook/angular-vite'
import { provideLocationMocks } from '@angular/common/testing'
import { provideRouter } from '@angular/router'
import { HttpErrorResponse } from '@angular/common/http'
import { NEVER, Observable, map, of, throwError, timer } from 'rxjs'
import { expect, fn, userEvent, waitFor, within } from 'storybook/test'
import { CatalogService } from '../application/catalog.service'
import { CatalogSuggestion, SuggestOptions } from '../domain/catalog.entity'
import { catalogSuggestion, dockerSuggestion } from '../testing/catalog.fixtures'
import { CurrentUrl } from '../testing/current-url'
import { SuggestSearchBox } from './suggest-search-box'

const SUGGESTIONS = [
  catalogSuggestion({ name: 'left-pad' }),
  catalogSuggestion({
    name: 'left-pad-extra',
    repository: { name: 'ui-kit' },
    owner: { kind: 'organization', slug: 'acme', display_name: 'Acme Corp' },
  }),
  dockerSuggestion({ name: 'left-proxy' }),
]

const searched = fn()

function suggesting(
  suggest: (text: string, options?: SuggestOptions) => Observable<CatalogSuggestion[]>,
): Decorator {
  return moduleMetadata({ providers: [{ provide: CatalogService, useValue: { suggest } }] })
}

const meta: Meta<SuggestSearchBox> = {
  title: 'Public/Catalog/SuggestSearchBox',
  component: SuggestSearchBox,
  args: { label: 'Rechercher un paquet', placeholder: 'Nom, description ou mot-clé' },
  decorators: [
    applicationConfig({
      providers: [provideRouter([{ path: '**', children: [] }]), provideLocationMocks()],
    }),
    moduleMetadata({ imports: [CurrentUrl] }),
  ],
  render: (args) => ({
    props: { ...args, onSearch: searched },
    template: `
      <div style="min-height: 20rem; max-width: 32rem">
        <app-suggest-search-box
          [label]="label"
          [placeholder]="placeholder"
          [labelHidden]="labelHidden"
          [compact]="compact"
          [format]="format"
          [owner]="owner"
          (search)="onSearch($event)"
        />
        <app-story-current-url />
      </div>`,
  }),
}
export default meta

type Story = StoryObj<SuggestSearchBox>

async function typeIn(canvasElement: HTMLElement, text: string) {
  const input = within(canvasElement).getByRole('combobox', { name: 'Rechercher un paquet' })
  await userEvent.type(input, text)
  return input
}

/** Nothing typed yet: the popup is closed. */
export const Idle: Story = {
  decorators: [suggesting(() => of(SUGGESTIONS))],
  play: async ({ canvasElement }) => {
    const input = within(canvasElement).getByRole('combobox', { name: 'Rechercher un paquet' })
    expect(input).toHaveAttribute('aria-expanded', 'false')
    expect(input).toHaveAttribute('aria-autocomplete', 'list')
    expect(input).not.toHaveAttribute('aria-activedescendant')
    expect(within(canvasElement).queryByRole('option')).toBeNull()
  },
}

/** The compact form used in the page header, with the label kept for screen readers only. */
export const Compact: Story = {
  args: { compact: true, labelHidden: true, placeholder: 'Rechercher un paquet…' },
  decorators: [suggesting(() => of(SUGGESTIONS))],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(canvas.getByRole('combobox', { name: 'Rechercher un paquet' })).toBeInTheDocument()
    expect(canvas.getByText('Rechercher un paquet', { selector: 'label' })).toHaveClass('sr-only')
  },
}

/** Typing from 2 characters shows format, name and owner of each suggestion. */
export const SuggestionsOpen: Story = {
  decorators: [suggesting(() => of(SUGGESTIONS))],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)

    const input = await typeIn(canvasElement, 'left')

    const options = await canvas.findAllByRole('option')
    expect(options).toHaveLength(3)
    expect(input).toHaveAttribute('aria-expanded', 'true')
    expect(within(options[1]).getByText('par Acme Corp / ui-kit')).toBeInTheDocument()
    expect(within(options[2]).getByText('Docker')).toBeInTheDocument()
    expect(canvas.getByText('3 suggestions disponibles')).toBeInTheDocument()
  },
}

/** Arrow keys move the highlight, which the input points at through aria-activedescendant. */
export const KeyboardHighlight: Story = {
  decorators: [suggesting(() => of(SUGGESTIONS))],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const input = await typeIn(canvasElement, 'left')
    const options = await canvas.findAllByRole('option')

    await userEvent.keyboard('{ArrowDown}{ArrowDown}')

    expect(input).toHaveAttribute('aria-activedescendant', options[1].id)
    expect(options[1]).toHaveAttribute('aria-selected', 'true')
    expect(options[0]).toHaveAttribute('aria-selected', 'false')
  },
}

/** Nothing matches: a short message instead of a list. */
export const NoSuggestion: Story = {
  decorators: [suggesting(() => of([]))],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)

    const input = await typeIn(canvasElement, 'zzz')

    expect(
      await canvas.findByText('Aucune suggestion', { selector: '.suggest__empty' }),
    ).toBeVisible()
    expect(input).toHaveAttribute('aria-expanded', 'false')
    expect(canvas.queryByRole('option')).toBeNull()
  },
}

/** A slow answer never blocks typing: the popup simply opens once it arrives. */
export const LoadingSlow: Story = {
  decorators: [suggesting(() => timer(1200).pipe(map(() => SUGGESTIONS)))],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)

    const input = await typeIn(canvasElement, 'left')
    await new Promise((resolve) => setTimeout(resolve, 400))

    expect(input).toHaveAttribute('aria-expanded', 'false')
    await userEvent.type(input, '-')
    expect(input).toHaveValue('left-')
    expect(await canvas.findAllByRole('option', {}, { timeout: 4000 })).toHaveLength(3)
  },
}

/** A pending request that never answers leaves the popup closed and the input usable. */
export const NeverAnswers: Story = {
  decorators: [suggesting(() => NEVER)],
  play: async ({ canvasElement }) => {
    const input = await typeIn(canvasElement, 'left')
    await new Promise((resolve) => setTimeout(resolve, 400))

    expect(input).toHaveAttribute('aria-expanded', 'false')
    expect(input).toBeEnabled()
  },
}

/** A failed request closes the popup without any message, and typing goes on. */
export const RequestFailed: Story = {
  decorators: [suggesting(() => throwError(() => new HttpErrorResponse({ status: 500 })))],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    searched.mockClear()

    const input = await typeIn(canvasElement, 'left')
    await new Promise((resolve) => setTimeout(resolve, 400))

    expect(input).toHaveAttribute('aria-expanded', 'false')
    expect(canvas.queryByRole('alert')).toBeNull()
    expect(canvas.queryByRole('option')).toBeNull()

    await userEvent.type(input, '-pad{Enter}')
    expect(searched).toHaveBeenCalledWith('left-pad')
  },
}

/** A 503 says the catalog is busy instead of looking like "no match". */
export const CatalogBusy: Story = {
  decorators: [suggesting(() => throwError(() => new HttpErrorResponse({ status: 503 })))],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)

    await typeIn(canvasElement, 'left')

    expect(
      await canvas.findByText('Le catalogue est momentanément occupé', {
        selector: '.suggest__empty',
      }),
    ).toBeVisible()
    expect(canvas.queryByText('Aucune suggestion')).toBeNull()
  },
}

/** Down, up and wrap-around, then Enter opens the highlighted package. */
export const ArrowKeysThenEnter: Story = {
  decorators: [suggesting(() => of(SUGGESTIONS))],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const input = await typeIn(canvasElement, 'left')
    const options = await canvas.findAllByRole('option')

    await userEvent.keyboard('{ArrowUp}')
    expect(input).toHaveAttribute('aria-activedescendant', options[2].id)
    await userEvent.keyboard('{ArrowDown}')
    expect(input).toHaveAttribute('aria-activedescendant', options[0].id)
    await userEvent.keyboard('{ArrowDown}{Enter}')

    await waitFor(() =>
      expect(canvas.getByLabelText('URL courante')).toHaveTextContent(
        '/o/acme/ui-kit/packages/npm/left-pad-extra',
      ),
    )
    expect(input).toHaveAttribute('aria-expanded', 'false')
  },
}

/** Enter without a highlighted suggestion hands the text to the host for a full search. */
export const EnterSearches: Story = {
  decorators: [suggesting(() => of(SUGGESTIONS))],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    searched.mockClear()
    const input = await typeIn(canvasElement, 'left')
    await canvas.findAllByRole('option')

    await userEvent.keyboard('{Enter}')

    expect(searched).toHaveBeenCalledTimes(1)
    expect(searched).toHaveBeenCalledWith('left')
    expect(input).toHaveAttribute('aria-expanded', 'false')
    expect(canvas.getByLabelText('URL courante')).toHaveTextContent(/^\/$/)
  },
}

/** Escape closes the popup and keeps the text. */
export const EscapeCloses: Story = {
  decorators: [suggesting(() => of(SUGGESTIONS))],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const input = await typeIn(canvasElement, 'left')
    await canvas.findAllByRole('option')
    await userEvent.keyboard('{ArrowDown}')

    await userEvent.keyboard('{Escape}')

    expect(input).toHaveAttribute('aria-expanded', 'false')
    expect(input).not.toHaveAttribute('aria-activedescendant')
    expect(input).toHaveValue('left')

    await userEvent.keyboard('{Escape}')
    expect(input).toHaveValue('left')
  },
}

/** Clicking a suggestion opens its package page. */
export const ClickOpensPackage: Story = {
  decorators: [suggesting(() => of(SUGGESTIONS))],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await typeIn(canvasElement, 'left')

    await userEvent.click((await canvas.findAllByRole('option'))[0])

    await waitFor(() =>
      expect(canvas.getByLabelText('URL courante')).toHaveTextContent(
        '/@admin/test-npm/packages/npm/left-pad',
      ),
    )
  },
}

/** Moving focus away closes the popup. */
export const BlurCloses: Story = {
  decorators: [suggesting(() => of(SUGGESTIONS))],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const input = await typeIn(canvasElement, 'left')
    await canvas.findAllByRole('option')

    await userEvent.tab()

    expect(input).toHaveAttribute('aria-expanded', 'false')
  },
}

/** Stands in for the server, which applies the format and owner filters itself. */
function filteringServer() {
  return fn((_text: string, options?: SuggestOptions) =>
    of(
      SUGGESTIONS.filter(
        (s) =>
          (!options?.format || s.kind === options.format) &&
          (!options?.owner ||
            (s.owner.kind === options.owner.kind && s.owner.slug === options.owner.slug)),
      ),
    ),
  )
}

const lockedFormatServer = filteringServer()

/** On a catalog page the format is sent to the server, which only answers with that format. */
export const LockedFormat: Story = {
  args: { format: 'docker' },
  decorators: [suggesting(lockedFormatServer)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await typeIn(canvasElement, 'left')

    expect(await canvas.findAllByRole('option')).toHaveLength(1)
    expect(canvas.getByText('left-proxy')).toBeInTheDocument()
    expect(canvas.getByText('1 suggestion disponible')).toBeInTheDocument()
    expect(lockedFormatServer).toHaveBeenCalledWith(
      'left',
      expect.objectContaining({ format: 'docker' }),
    )
  },
}

const lockedOwnerServer = filteringServer()

/**
 * On an owner page the owner is sent to the server, which only answers with that owner's packages.
 */
export const LockedOwner: Story = {
  args: { owner: { kind: 'organization', slug: 'acme' } },
  decorators: [suggesting(lockedOwnerServer)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await typeIn(canvasElement, 'left')

    const options = await canvas.findAllByRole('option')
    expect(options.map((option) => option.querySelector('.suggest__name')?.textContent)).toEqual([
      'left-pad-extra',
      'left-proxy',
    ])
    expect(lockedOwnerServer).toHaveBeenCalledWith(
      'left',
      expect.objectContaining({ owner: { kind: 'organization', slug: 'acme' } }),
    )
  },
}
