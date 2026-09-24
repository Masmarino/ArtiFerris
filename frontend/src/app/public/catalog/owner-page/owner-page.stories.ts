import {
  applicationConfig,
  moduleMetadata,
  type Decorator,
  type Meta,
  type StoryObj,
} from '@storybook/angular-vite'
import { HttpErrorResponse } from '@angular/common/http'
import { provideRouter } from '@angular/router'
import { expect, fn, userEvent, within } from 'storybook/test'
import { NEVER, Observable, of, throwError } from 'rxjs'
import { AuthService } from '../../../auth/application/auth.service'
import { CatalogService } from '../application/catalog.service'
import { CatalogQuery, OwnerSummary } from '../domain/catalog.entity'
import { catalogEntry, dockerEntry, ownerSummary, searchResult } from '../testing/catalog.fixtures'
import { fakeUrlProviders } from '../testing/fake-url'
import { OwnerPage } from './owner-page'

const search = fn((query: CatalogQuery) =>
  of(
    searchResult(
      [catalogEntry(), dockerEntry()].filter(
        (entry) => !query.format || entry.kind === query.format,
      ),
    ),
  ),
)

function scenario(
  pathParams: Record<string, string>,
  owner: () => Observable<OwnerSummary | null>,
): Decorator[] {
  return [
    applicationConfig({ providers: fakeUrlProviders({}, {}, pathParams) }),
    moduleMetadata({ providers: [{ provide: CatalogService, useValue: { owner, search } }] }),
  ]
}

const meta: Meta<OwnerPage> = {
  title: 'Public/Catalog/OwnerPage',
  component: OwnerPage,
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

type Story = StoryObj<OwnerPage>

/** `/@alice`: display name, handle, counts, then the owner's entries. */
export const PersonalOwner: Story = {
  decorators: scenario({ username: 'alice' }, () =>
    of(ownerSummary({ slug: 'alice', display_name: 'Alice Martin' })),
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByRole('heading', { level: 1 })).toHaveTextContent('Alice Martin')
    expect(canvas.getByText('@alice')).toBeInTheDocument()
    expect(canvas.getByText('2 dépôts · 3 paquets · 4 images')).toBeInTheDocument()
    expect(await canvas.findByRole('link', { name: 'hangar-demo' })).toBeInTheDocument()
    expect(search).toHaveBeenLastCalledWith(
      expect.objectContaining({ owner: { kind: 'personal', slug: 'alice' } }),
    )
  },
}

/** `/o/acme`: no handle, the search is restricted to the organization. */
export const Organization: Story = {
  decorators: scenario({ slug: 'acme' }, () =>
    of(ownerSummary({ kind: 'organization', slug: 'acme', display_name: 'Acme Corp' })),
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByRole('heading', { level: 1 })).toHaveTextContent('Acme Corp')
    expect(canvas.getByText('Organisation')).toBeInTheDocument()
    expect(canvas.queryByText(/^@/)).toBeNull()
    expect(await canvas.findByRole('link', { name: 'team/api' })).toBeInTheDocument()
    expect(search).toHaveBeenLastCalledWith(
      expect.objectContaining({ owner: { kind: 'organization', slug: 'acme' } }),
    )
  },
}

/** Only repositories so far: the zero counts are left out. */
export const OnlyRepositories: Story = {
  decorators: scenario({ username: 'alice' }, () =>
    of(ownerSummary({ slug: 'alice', repository_count: 1, package_count: 0, image_count: 0 })),
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByText('1 dépôt')).toBeInTheDocument()
    expect(canvas.queryByText(/paquet|image/, { selector: '.owner-page__counts' })).toBeNull()
  },
}

/** The format filter narrows the owner's entries. */
export const FilteredByFormat: Story = {
  decorators: scenario({ username: 'alice' }, () => of(ownerSummary({ slug: 'alice' }))),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await canvas.findByRole('link', { name: 'team/api' })

    await userEvent.click(canvas.getByRole('radio', { name: 'npm' }))

    expect(await canvas.findByRole('link', { name: 'hangar-demo' })).toBeInTheDocument()
    expect(search).toHaveBeenLastCalledWith(
      expect.objectContaining({ format: 'npm', owner: { kind: 'personal', slug: 'alice' } }),
    )
  },
}

export const Loading: Story = {
  decorators: scenario({ username: 'alice' }, () => NEVER),
  play: async ({ canvasElement }) => {
    expect(await within(canvasElement).findByRole('status')).toHaveTextContent('Chargement…')
  },
}

/** Nothing public, or no such account: the same answer for both. */
export const NotFound: Story = {
  decorators: scenario({ username: 'ghost' }, () => of(null)),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByText('Propriétaire introuvable')).toBeInTheDocument()
    expect(canvas.getByText("Il n'y a rien de public à cette adresse.")).toBeInTheDocument()
    expect(canvas.getByRole('link', { name: 'Parcourir le catalogue public' })).toHaveAttribute(
      'href',
      expect.stringMatching(/\/explorer$/),
    )
    expect(canvas.queryByRole('heading', { level: 1 })).toBeNull()
  },
}

let attempts = 0

/** A failed load can be retried. */
export const ErrorWithRetry: Story = {
  decorators: scenario({ username: 'alice' }, () =>
    ++attempts % 2 === 1
      ? throwError(() => new HttpErrorResponse({ status: 500 }))
      : of(ownerSummary({ slug: 'alice' })),
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByRole('alert')).toHaveTextContent('Échec du chargement')

    await userEvent.click(canvas.getByRole('button', { name: 'Réessayer' }))

    expect(await canvas.findByRole('heading', { level: 1 })).toHaveTextContent('admin')
    expect(canvas.queryByRole('alert')).toBeNull()
  },
}

export const RateLimited: Story = {
  decorators: scenario({ username: 'alice' }, () =>
    throwError(() => new HttpErrorResponse({ status: 429 })),
  ),
  play: async ({ canvasElement }) => {
    expect(await within(canvasElement).findByRole('alert')).toHaveTextContent('Trop de requêtes')
  },
}
