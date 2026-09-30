import {
  applicationConfig,
  moduleMetadata,
  type Decorator,
  type Meta,
  type StoryObj,
} from '@storybook/angular-vite'
import { provideRouter } from '@angular/router'
import { HttpErrorResponse } from '@angular/common/http'
import { expect, userEvent, within } from 'storybook/test'
import { NEVER, Observable, of, throwError } from 'rxjs'
import { AuthService } from '../../../auth/application/auth.service'
import { CatalogService } from '../application/catalog.service'
import { CatalogInfo, CatalogQuery, CatalogSearchResult } from '../domain/catalog.entity'
import { CATALOG_INFOS, catalogEntry, dockerEntry, searchResult } from '../testing/catalog.fixtures'
import { fakeUrlProviders } from '../testing/fake-url'
import { ExplorerPage } from './explorer-page'

const RESULTS = [catalogEntry(), dockerEntry()]

function scenario(
  catalogs: () => Observable<CatalogInfo[]>,
  search: (query: CatalogQuery) => Observable<CatalogSearchResult> = () =>
    of(searchResult(RESULTS)),
): Decorator[] {
  return [
    applicationConfig({ providers: fakeUrlProviders() }),
    moduleMetadata({ providers: [{ provide: CatalogService, useValue: { catalogs, search } }] }),
  ]
}

const meta: Meta<ExplorerPage> = {
  title: 'Public/Catalog/ExplorerPage',
  component: ExplorerPage,
  decorators: [
    applicationConfig({
      providers: [
        provideRouter([]),
        { provide: AuthService, useValue: { isAuthenticated: () => false } },
      ],
    }),
  ],
}
export default meta

type Story = StoryObj<ExplorerPage>

/** One card per catalog, then the search across every format. */
export const Default: Story = {
  decorators: scenario(() => of(CATALOG_INFOS)),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByRole('heading', { level: 1 })).toHaveTextContent(
      'Explorer les paquets publics',
    )
    const npm = await canvas.findByRole('link', { name: /artiferris-npm/ })
    expect(npm).toHaveTextContent('12 paquets')
    expect(npm).toHaveAttribute('href', expect.stringMatching(/\/artiferris-npm$/))
    expect(canvas.getByRole('link', { name: /artiferris-docker/ })).toHaveTextContent('1 image')
    expect(await canvas.findByRole('link', { name: 'hangar-demo' })).toBeInTheDocument()
    expect(canvas.getByRole('radio', { name: 'Docker' })).toBeInTheDocument()
    expect(canvas.queryByRole('heading', { name: 'Populaires cette semaine' })).toBeNull()
  },
}

const POPULAR = [
  catalogEntry({ name: '@acme/button', downloads_7d: 12_408 }),
  dockerEntry({ downloads_7d: 350 }),
  catalogEntry({ name: 'left-pad', downloads_7d: 1 }),
]

const popularOrRecent = (query: CatalogQuery) =>
  of(searchResult(query.sort === 'popular' ? POPULAR : RESULTS))

/** The most downloaded entries of the week sit above the recent listing. */
export const PopularThisWeek: Story = {
  decorators: scenario(() => of(CATALOG_INFOS), popularOrRecent),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const heading = await canvas.findByRole('heading', { name: 'Populaires cette semaine' })
    const section = heading.closest('section')!
    expect(within(section).getAllByRole('heading', { level: 3 })).toHaveLength(3)
    expect(within(section).getByText(/^12\s408 téléchargements cette semaine$/)).toBeInTheDocument()
    expect(within(section).getByText('1 téléchargement cette semaine')).toBeInTheDocument()
    expect(await canvas.findByRole('heading', { name: 'Récemment mis à jour' })).toBeInTheDocument()
  },
}

/** The popular section has its own spinner and never blocks the rest. */
export const PopularLoading: Story = {
  decorators: scenario(
    () => of(CATALOG_INFOS),
    (query) => (query.sort === 'popular' ? NEVER : of(searchResult(RESULTS))),
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByText('Chargement des paquets populaires…')).toBeInTheDocument()
    expect(await canvas.findByRole('link', { name: 'hangar-demo' })).toBeInTheDocument()
  },
}

let popularAttempts = 0

/**
 * A failing popular section reports it on its own, keeps the search working, and can be retried.
 */
export const PopularFailed: Story = {
  decorators: scenario(
    () => of(CATALOG_INFOS),
    (query) =>
      query.sort !== 'popular'
        ? of(searchResult(RESULTS))
        : ++popularAttempts % 2 === 1
          ? throwError(() => new HttpErrorResponse({ status: 500 }))
          : of(searchResult(POPULAR)),
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(
      await canvas.findByText('Échec du chargement des paquets populaires.'),
    ).toBeInTheDocument()
    expect(await canvas.findByRole('link', { name: 'hangar-demo' })).toBeInTheDocument()

    await userEvent.click(canvas.getByRole('button', { name: 'Réessayer' }))

    expect(await canvas.findByText(/^12\s408 téléchargements cette semaine$/)).toBeInTheDocument()
    expect(canvas.queryByText('Échec du chargement des paquets populaires.')).toBeNull()
  },
}

export const CatalogsLoading: Story = {
  decorators: scenario(() => NEVER),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByText('Chargement des catalogues…')).toBeInTheDocument()
    expect(await canvas.findByRole('link', { name: 'hangar-demo' })).toBeInTheDocument()
  },
}

let attempts = 0

/** The catalog cards can fail on their own without taking the search down, and be retried. */
export const CatalogsFailed: Story = {
  decorators: scenario(() =>
    ++attempts % 2 === 1
      ? throwError(() => new HttpErrorResponse({ status: 500 }))
      : of(CATALOG_INFOS),
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByText('Échec du chargement des catalogues.')).toBeInTheDocument()
    expect(await canvas.findByRole('link', { name: 'hangar-demo' })).toBeInTheDocument()

    await userEvent.click(canvas.getByRole('button', { name: 'Réessayer' }))

    expect(await canvas.findByRole('link', { name: /artiferris-npm/ })).toBeInTheDocument()
    expect(canvas.queryByText('Échec du chargement des catalogues.')).toBeNull()
  },
}

/** A fresh instance: empty catalogs and a welcome state instead of results. */
export const EmptyInstance: Story = {
  decorators: scenario(
    () => of(CATALOG_INFOS.map((catalog) => ({ ...catalog, entry_count: 0 }))),
    () => of(searchResult([])),
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByRole('link', { name: /artiferris-npm/ })).toHaveTextContent(
      '0 paquet',
    )
    expect(await canvas.findByText("Rien à explorer pour l'instant")).toBeInTheDocument()
  },
}
