import {
  applicationConfig,
  moduleMetadata,
  type Decorator,
  type Meta,
  type StoryObj,
} from '@storybook/angular-vite'
import { provideRouter } from '@angular/router'
import { HttpErrorResponse } from '@angular/common/http'
import { expect, fn, userEvent, waitFor, within } from 'storybook/test'
import { NEVER, Observable, of, throwError } from 'rxjs'
import { CatalogService } from '../application/catalog.service'
import {
  CatalogQuery,
  CatalogSearchResult,
  CatalogSuggestion,
  SuggestOptions,
} from '../domain/catalog.entity'
import {
  CATALOG_INFOS,
  catalogEntry,
  catalogSuggestion,
  dockerEntry,
  dockerSuggestion,
  searchResult,
} from '../testing/catalog.fixtures'
import { fakeUrlProviders } from '../testing/fake-url'
import { CatalogSearch } from './catalog-search'

const RESULTS = [
  catalogEntry(),
  catalogEntry({
    name: '@acme/button',
    description: 'Accessible button component.',
    keywords: ['ui', 'button'],
    latest: '2.1.0',
    updated_at: '2026-09-20T08:00:00Z',
    owner: { kind: 'organization', slug: 'acme', display_name: 'Acme Corp' },
    repository: { name: 'ui-kit' },
    registry_url: 'http://localhost:4200/npm/o/acme/ui-kit/',
  }),
  dockerEntry(),
]

/**
 * A fake URL plus a fake service answering with `search` and `suggest`; every story declares its
 * own so nothing leaks between them.
 */
function scenario(
  params: Record<string, string>,
  search: (query: CatalogQuery) => Observable<CatalogSearchResult>,
  suggest: (text: string, options?: SuggestOptions) => Observable<CatalogSuggestion[]> = () =>
    of([]),
): Decorator[] {
  return [
    applicationConfig({ providers: fakeUrlProviders(params) }),
    moduleMetadata({ providers: [{ provide: CatalogService, useValue: { search, suggest } }] }),
  ]
}

const meta: Meta<CatalogSearch> = {
  title: 'Public/Catalog/CatalogSearch',
  component: CatalogSearch,
  decorators: [applicationConfig({ providers: [provideRouter([])] })],
}
export default meta

type Story = StoryObj<CatalogSearch>

/** No text: the most recently updated items. */
export const RecentItems: Story = {
  decorators: scenario({}, () => of(searchResult(RESULTS))),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByRole('heading', { name: 'Récemment mis à jour' })).toBeInTheDocument()
    expect(await canvas.findByRole('link', { name: 'hangar-demo' })).toBeInTheDocument()
    expect(canvas.getByRole('link', { name: '@acme/button' })).toBeInTheDocument()
    expect(canvas.getByRole('link', { name: 'team/api' })).toBeInTheDocument()
    expect(canvasElement.querySelector('.catalog-search__count')).toHaveTextContent('3 résultats')
    expect(canvas.getByRole('radio', { name: 'Récents' })).toBeChecked()
  },
}

const POPULAR_RESULTS = [
  { ...RESULTS[1], downloads_7d: 12_408 },
  { ...RESULTS[0], downloads_7d: 350 },
  { ...RESULTS[2], downloads_7d: 1 },
]

const popularSearch = fn((query: CatalogQuery) =>
  of(searchResult(query.sort === 'popular' ? POPULAR_RESULTS : RESULTS)),
)

/**
 * Without text, sort=popular in the URL lists the most downloaded first, with their weekly
 * downloads.
 */
export const PopularItems: Story = {
  decorators: scenario({ sort: 'popular' }, popularSearch),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(
      await canvas.findByRole('heading', { name: 'Populaires cette semaine' }),
    ).toBeInTheDocument()
    expect(canvas.getByRole('radio', { name: 'Populaires' })).toBeChecked()
    expect(canvas.getByRole('radio', { name: 'Pertinence' })).toBeDisabled()
    expect(await canvas.findByText(/^12\s408 téléchargements cette semaine$/)).toBeInTheDocument()
    expect(canvas.getByText('350 téléchargements cette semaine')).toBeInTheDocument()
    expect(canvas.getByText('1 téléchargement cette semaine')).toBeInTheDocument()
    expect(popularSearch).toHaveBeenLastCalledWith(expect.objectContaining({ sort: 'popular' }))
  },
}

const pickPopularSearch = fn((query: CatalogQuery) =>
  of(searchResult(query.sort === 'popular' ? POPULAR_RESULTS : RESULTS, { total: 45 })),
)

/** Picking Populaires re-runs the search and sends the reader back to the first page. */
export const PickPopularSort: Story = {
  decorators: scenario({ page: '2' }, pickPopularSearch),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await canvas.findByRole('heading', { name: 'Récemment mis à jour' })

    await userEvent.click(canvas.getByRole('radio', { name: 'Populaires' }))

    await waitFor(() =>
      expect(pickPopularSearch).toHaveBeenLastCalledWith(
        expect.objectContaining({ sort: 'popular', page: 1 }),
      ),
    )
    expect(
      await canvas.findByRole('heading', { name: 'Populaires cette semaine' }),
    ).toBeInTheDocument()
    expect(canvas.getByText('Page 1 sur 3')).toBeInTheDocument()
  },
}

/** With text, the popular sort keeps the results heading and ranks the matches by downloads. */
export const PopularSortWithText: Story = {
  decorators: scenario({ q: 'demo', sort: 'popular' }, popularSearch),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(
      await canvas.findByRole('heading', { name: 'Résultats pour « demo »' }),
    ).toBeInTheDocument()
    expect(canvas.getByRole('radio', { name: 'Populaires' })).toBeChecked()
    expect(canvas.getByRole('radio', { name: 'Pertinence' })).not.toBeDisabled()
  },
}

const typedSearch = fn((query: CatalogQuery) =>
  of(searchResult(query.q ? RESULTS.slice(0, 1) : RESULTS)),
)

/** Typing searches after a short pause. */
export const DebouncedTyping: Story = {
  decorators: scenario({}, typedSearch),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await canvas.findByRole('link', { name: '@acme/button' })

    await userEvent.type(canvas.getByLabelText('Rechercher un paquet'), 'demo')

    await waitFor(() =>
      expect(typedSearch).toHaveBeenLastCalledWith(expect.objectContaining({ q: 'demo' })),
    )
    expect(
      await canvas.findByRole('heading', { name: 'Résultats pour « demo »' }),
    ).toBeInTheDocument()
    await waitFor(() => expect(canvas.queryByRole('link', { name: '@acme/button' })).toBeNull())
    expect(canvas.getByRole('radio', { name: 'Pertinence' })).toBeChecked()
  },
}

const enterSearch = fn((query: CatalogQuery) =>
  of(searchResult(query.q ? RESULTS.slice(2) : RESULTS)),
)

/** Enter searches right away. */
export const SearchOnEnter: Story = {
  decorators: scenario({}, enterSearch),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await canvas.findByRole('link', { name: '@acme/button' })

    await userEvent.type(canvas.getByLabelText('Rechercher un paquet'), 'team{Enter}')

    await waitFor(() =>
      expect(enterSearch).toHaveBeenLastCalledWith(expect.objectContaining({ q: 'team' })),
    )
    await waitFor(() => expect(canvas.queryByRole('link', { name: '@acme/button' })).toBeNull())
    expect(canvas.getByRole('link', { name: 'team/api' })).toBeInTheDocument()
  },
}

const formatSearch = fn((query: CatalogQuery) =>
  of(searchResult(RESULTS.filter((entry) => !query.format || entry.kind === query.format))),
)

/** The format filter narrows the results. */
export const FormatFilter: Story = {
  args: { catalogs: CATALOG_INFOS },
  decorators: scenario({}, formatSearch),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await canvas.findByRole('link', { name: 'team/api' })

    await userEvent.click(canvas.getByRole('radio', { name: 'Docker' }))

    await waitFor(() =>
      expect(formatSearch).toHaveBeenLastCalledWith(expect.objectContaining({ format: 'docker' })),
    )
    await waitFor(() => expect(canvas.queryByRole('link', { name: 'hangar-demo' })).toBeNull())
    expect(canvas.getByRole('radio', { name: 'Docker' })).toBeChecked()

    await userEvent.click(canvas.getByRole('radio', { name: 'Tous' }))
    expect(await canvas.findByRole('link', { name: 'hangar-demo' })).toBeInTheDocument()
  },
}

const lockedSearch = fn((query: CatalogQuery) =>
  of(searchResult(RESULTS.filter((entry) => entry.kind === query.format))),
)

/** On a catalog page the format is fixed: no format filter, and only that format comes back. */
export const LockedFormat: Story = {
  args: { format: 'npm' },
  decorators: scenario({ format: 'docker' }, lockedSearch),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByRole('link', { name: 'hangar-demo' })).toBeInTheDocument()
    expect(canvas.queryByRole('radiogroup', { name: 'Format' })).toBeNull()
    expect(canvas.getByRole('radiogroup', { name: 'Trier par' })).toBeInTheDocument()
    expect(canvas.queryByRole('link', { name: 'team/api' })).toBeNull()
    expect(lockedSearch).toHaveBeenLastCalledWith(expect.objectContaining({ format: 'npm' }))
  },
}

const ownerSearch = fn((query: CatalogQuery) =>
  of(searchResult(RESULTS.filter((entry) => entry.owner.slug === query.owner?.slug))),
)

/** On an owner page every search is restricted to that owner, and the format filter stays. */
export const LockedOwner: Story = {
  args: { owner: { kind: 'organization', slug: 'acme' }, catalogs: CATALOG_INFOS },
  decorators: scenario({}, ownerSearch),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByRole('link', { name: '@acme/button' })).toBeInTheDocument()
    expect(canvas.getByRole('link', { name: 'team/api' })).toBeInTheDocument()
    expect(canvas.queryByRole('link', { name: 'hangar-demo' })).toBeNull()
    expect(canvas.getByRole('radiogroup', { name: 'Format' })).toBeInTheDocument()
    expect(ownerSearch).toHaveBeenLastCalledWith(
      expect.objectContaining({ owner: { kind: 'organization', slug: 'acme' } }),
    )
  },
}

/** The owner has entries, just none in the chosen format. */
export const LockedOwnerNothingInFormat: Story = {
  args: { owner: { kind: 'personal', slug: 'admin' }, catalogs: CATALOG_INFOS },
  decorators: scenario({ format: 'docker' }, () => of(searchResult([]))),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(
      await canvas.findByText("Ce propriétaire n'a rien de public à afficher ici."),
    ).toBeInTheDocument()
    expect(canvas.queryByText(/Rendez un dépôt public/)).toBeNull()
    expect(canvas.getByRole('button', { name: 'Tous les formats' })).toBeInTheDocument()
  },
}

const pagedSearch = fn((query: CatalogQuery) =>
  of(searchResult(RESULTS.slice(0, 2), { total: 45, page: query.page, per_page: 20 })),
)

/** Previous and next stay within the first and last page. */
export const Pagination: Story = {
  decorators: scenario({}, pagedSearch),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByText('Page 1 sur 3')).toBeInTheDocument()
    expect(canvas.getByRole('button', { name: 'Précédent' })).toBeDisabled()

    await userEvent.click(canvas.getByRole('button', { name: 'Suivant' }))
    expect(await canvas.findByText('Page 2 sur 3')).toBeInTheDocument()
    expect(pagedSearch).toHaveBeenLastCalledWith(expect.objectContaining({ page: 2 }))

    await userEvent.click(canvas.getByRole('button', { name: 'Suivant' }))
    expect(await canvas.findByText('Page 3 sur 3')).toBeInTheDocument()
    expect(canvas.getByRole('button', { name: 'Suivant' })).toBeDisabled()
    expect(canvas.getByRole('heading', { level: 2 })).toHaveFocus()
  },
}

export const Loading: Story = {
  decorators: scenario({ q: 'demo' }, () => NEVER),
  play: async ({ canvasElement }) => {
    expect(await within(canvasElement).findByRole('status')).toHaveTextContent(
      'Recherche en cours…',
    )
  },
}

let attempts = 0

/** A failed search can be retried. */
export const ErrorWithRetry: Story = {
  decorators: scenario({}, () =>
    ++attempts % 2 === 1
      ? throwError(() => new HttpErrorResponse({ status: 500 }))
      : of(searchResult(RESULTS)),
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByRole('alert')).toHaveTextContent('La recherche a échoué')

    await userEvent.click(canvas.getByRole('button', { name: 'Réessayer' }))

    expect(await canvas.findByRole('link', { name: 'hangar-demo' })).toBeInTheDocument()
    expect(canvas.queryByRole('alert')).toBeNull()
  },
}

export const CatalogBusy: Story = {
  decorators: scenario({}, () => throwError(() => new HttpErrorResponse({ status: 503 }))),
  play: async ({ canvasElement }) => {
    expect(await within(canvasElement).findByRole('alert')).toHaveTextContent(
      'Le catalogue est momentanément occupé',
    )
  },
}

export const RateLimited: Story = {
  decorators: scenario({}, () => throwError(() => new HttpErrorResponse({ status: 429 }))),
  play: async ({ canvasElement }) => {
    expect(await within(canvasElement).findByRole('alert')).toHaveTextContent('Trop de requêtes')
  },
}

/** Nothing matches the text: suggests another search and offers to start over. */
export const NoResults: Story = {
  decorators: scenario({ q: 'zzz' }, (query) => of(searchResult(query.q ? [] : RESULTS))),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(
      await canvas.findByText('Aucun résultat', { selector: '.gbt-empty-state__heading' }),
    ).toBeInTheDocument()
    expect(canvas.getByText(/Essayez un autre nom/)).toBeInTheDocument()

    await userEvent.click(canvas.getByRole('button', { name: 'Effacer la recherche' }))

    expect(await canvas.findByRole('link', { name: 'hangar-demo' })).toBeInTheDocument()
    expect(canvas.getByLabelText('Rechercher un paquet')).toHaveValue('')
  },
}

/** Nothing public at all yet. */
export const InstanceEmpty: Story = {
  decorators: scenario({}, () => of(searchResult([]))),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByText("Rien à explorer pour l'instant")).toBeInTheDocument()
    expect(canvas.queryByRole('button', { name: 'Effacer la recherche' })).toBeNull()
  },
}

const SUGGESTIONS = [
  catalogSuggestion(),
  catalogSuggestion({
    name: '@acme/button',
    repository: { name: 'ui-kit' },
    owner: { kind: 'organization', slug: 'acme', display_name: 'Acme Corp' },
  }),
  dockerSuggestion(),
]

/** Typing suggests package names in a popup under the search box. */
export const SuggestionsWhileTyping: Story = {
  decorators: scenario(
    {},
    () => of(searchResult(RESULTS)),
    () => of(SUGGESTIONS),
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await canvas.findByRole('link', { name: 'hangar-demo' })

    await userEvent.type(canvas.getByRole('combobox', { name: 'Rechercher un paquet' }), 'ha')

    expect(await canvas.findAllByRole('option')).toHaveLength(3)
    expect(canvas.getByRole('combobox')).toHaveAttribute('aria-expanded', 'true')
  },
}

/** On a catalog page the format goes to the server, so the suggestions stay within it. */
export const SuggestionsLockedFormat: Story = {
  args: { format: 'docker' },
  decorators: scenario(
    {},
    () => of(searchResult(RESULTS.slice(2))),
    (_text, options) =>
      of(SUGGESTIONS.filter((s) => !options?.format || s.kind === options.format)),
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await canvas.findByRole('link', { name: 'team/api' })

    await userEvent.type(canvas.getByRole('combobox', { name: 'Rechercher un paquet' }), 'te')

    const options = await canvas.findAllByRole('option')
    expect(options).toHaveLength(1)
    expect(options[0]).toHaveTextContent('team/api')
  },
}

const FUZZY_RESULTS = [
  catalogEntry({ name: 'lefft-pad', match_kind: 'fuzzy' }),
  dockerEntry({ name: 'team/lefft', match_kind: 'fuzzy' }),
]

/** Exact results first, typo-tolerant ones after them under their own heading. */
export const ApproximateResultsMixed: Story = {
  decorators: scenario({ q: 'left' }, () =>
    of(searchResult([catalogEntry({ name: 'left-pad', match_kind: 'prefix' }), ...FUZZY_RESULTS])),
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const heading = await canvas.findByRole('heading', { name: 'Résultats approchants' })

    expect(canvas.getByRole('link', { name: 'left-pad' })).toBeInTheDocument()
    expect(
      canvas.getByRole('link', { name: 'left-pad' }).compareDocumentPosition(heading) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy()
    expect(canvas.getByRole('link', { name: 'lefft-pad' })).toBeInTheDocument()
    expect(canvas.queryByText(/Aucun résultat exact/)).toBeNull()
  },
}

/** Nothing matches exactly: the hint explains why only approximate results show. */
export const ApproximateResultsOnly: Story = {
  decorators: scenario({ q: 'lefft' }, () => of(searchResult(FUZZY_RESULTS))),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)

    expect(
      await canvas.findByText('Aucun résultat exact — voici des résultats approchants.'),
    ).toBeInTheDocument()
    expect(canvas.getByRole('heading', { name: 'Résultats approchants' })).toBeInTheDocument()
    expect(canvas.getByRole('link', { name: 'lefft-pad' })).toBeInTheDocument()
    expect(canvas.getByRole('link', { name: 'team/lefft' })).toBeInTheDocument()
  },
}
