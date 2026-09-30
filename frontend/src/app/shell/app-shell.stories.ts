import {
  applicationConfig,
  moduleMetadata,
  type Meta,
  type StoryObj,
} from '@storybook/angular-vite'
import { provideRouter } from '@angular/router'
import { Component, signal } from '@angular/core'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { HttpErrorResponse } from '@angular/common/http'
import { of, throwError } from 'rxjs'
import { AppShell } from './app-shell'
import { AuthService } from '../auth/application/auth.service'
import { MeService } from './application/me.service'
import { ReadableCatalogService } from './application/readable-catalog.service'
import { VersionService } from './application/version.service'
import { RepositoriesService } from '../repositories/application/repositories.service'
import { UsersService } from '../users/application/users.service'
import type { MeResponse } from './domain/me.entity'
import type { ReadableCatalogEntry } from './domain/readable-catalog.entity'
import {
  proxiedEntry,
  readableDockerEntry,
  readableEntry,
  readableSearchResult,
} from './testing/readable-catalog.fixtures'
import type { RepositorySummary } from '../repositories/domain/repository.entity'
import type { UserSummary } from '../users/domain/user.entity'

const ME: MeResponse = {
  id: 'u1',
  username: 'florian',
  is_super_admin: true,
  is_organization_admin: false,
  organization_id: 'org-acme',
  created_at: '2026-01-01T00:00:00Z',
}

const REPO: RepositorySummary = {
  id: 'r1',
  name: 'artiferris-web',
  format: 'npm',
  repo_type: 'hosted',
  remote_url: null,
  remote_credentials_set: false,
  group_members: [],
  quota_bytes: null,
  retention_keep_last_n: null,
  is_public: false,
  my_role: 'admin',
  organization_id: 'org-acme',
  owner_name: 'Acme Corp',
  owner_is_personal: false,
}

const USER: UserSummary = {
  id: 'u2',
  username: 'acme-writer',
  is_super_admin: false,
  organization_id: 'org-acme',
  email: null,
  invitation_pending: false,
}

function fakeReadableCatalog(entries: ReadableCatalogEntry[]) {
  return { search: () => of(readableSearchResult(entries)) }
}

function fakeMe(overrides: {
  isSuperAdmin?: boolean
  isOrganizationAdmin?: boolean
  organizationId?: string | null
}): Partial<MeService> {
  return {
    username: signal('florian'),
    isSuperAdmin: signal(overrides.isSuperAdmin ?? true),
    isOrganizationAdmin: signal(overrides.isOrganizationAdmin ?? false),
    organizationId: signal(overrides.organizationId ?? null),
    load: () => of(ME),
  }
}

// Just enough of a route table that selecting a search result's real router.navigateByUrl()
// resolves instead of throwing NG04002 against an empty one — AppShell needs the real Router
// (not a stub) for its routerLink/routerLinkActive/router-outlet usage.
@Component({ standalone: true, template: '' })
class DummyRoutedComponent {}

const meta: Meta<AppShell> = {
  title: 'Shell/AppShell',
  component: AppShell,
  decorators: [
    applicationConfig({
      providers: [
        provideRouter([
          { path: 'repositories/:id', component: DummyRoutedComponent },
          {
            path: 'repositories/:id/packages/:format/:name',
            component: DummyRoutedComponent,
          },
          { path: 'users/:id', component: DummyRoutedComponent },
        ]),
      ],
    }),
    moduleMetadata({
      providers: [
        { provide: AuthService, useValue: { token: signal(null), logout: () => undefined } },
        { provide: MeService, useValue: fakeMe({}) },
        { provide: VersionService, useValue: { version: signal('0.4.6'), load: () => undefined } },
        { provide: RepositoriesService, useValue: { list: () => of([REPO]) } },
        { provide: UsersService, useValue: { list: () => of([USER]) } },
        {
          provide: ReadableCatalogService,
          useValue: fakeReadableCatalog([readableEntry(), readableDockerEntry()]),
        },
      ],
    }),
  ],
}
export default meta

type Story = StoryObj<AppShell>

/** A super-admin sees every nav item, the version footer, and their username. */
export const Default: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByText('florian')).toBeInTheDocument())
    expect(canvas.getByRole('link', { name: /Administration/ })).toBeInTheDocument()
    expect(canvas.getByRole('link', { name: /Utilisateurs/ })).toBeInTheDocument()
    expect(canvas.getByText('v0.4.6')).toBeInTheDocument()
  },
}

/** Collapsing the sidebar hides nav labels and the version footer, and swaps the org-brandable
 * logo for ArtiFerris's own square icon mark — but each nav link keeps its accessible name via
 * aria-label/title, and the toggle itself flips label. */
export const CollapsingTheSidebar: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByText('florian')).toBeInTheDocument())
    const toggle = canvas.getByRole('button', { name: 'Réduire le menu' })
    await userEvent.click(toggle)
    await waitFor(() =>
      expect(canvas.getByRole('button', { name: 'Déployer le menu' })).toBeInTheDocument(),
    )
    expect(canvas.getByRole('link', { name: 'Dépôts' })).toBeInTheDocument()
    expect(canvas.queryByText('Dépôts')).not.toBeInTheDocument()
    expect(canvas.queryByAltText('ArtiFerris logo')).not.toBeInTheDocument()
    expect(canvas.getByAltText('ArtiFerris')).toHaveAttribute('src', '/Logo.png')
    expect(canvas.queryByText('v0.4.6')).not.toBeInTheDocument()
  },
}

let sessionLoadCalls = 0

/** A transient failure of /api/me keeps the session and offers a retry instead of logging out. */
export const SessionLoadFailed: Story = {
  beforeEach: () => {
    sessionLoadCalls = 0
  },
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: MeService,
          useValue: {
            ...fakeMe({}),
            load: () =>
              sessionLoadCalls++ === 0
                ? throwError(() => new HttpErrorResponse({ status: 503 }))
                : of(ME),
          },
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByRole('alert')).toHaveTextContent(
      'Impossible de charger votre session',
    )
    expect(canvas.queryByRole('link', { name: /Dépôts/ })).not.toBeInTheDocument()

    await userEvent.click(canvas.getByRole('button', { name: 'Réessayer' }))

    await waitFor(() => expect(canvas.getByText('florian')).toBeInTheDocument())
    expect(canvas.queryByRole('alert')).not.toBeInTheDocument()
  },
}

/** A plain member never sees Administration or Utilisateurs — staff/super-admin only. */
export const AsPlainMember: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: MeService,
          useValue: fakeMe({ isSuperAdmin: false, isOrganizationAdmin: false }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByText('florian')).toBeInTheDocument())
    expect(canvas.queryByRole('link', { name: /Administration/ })).not.toBeInTheDocument()
    expect(canvas.queryByRole('link', { name: /Utilisateurs/ })).not.toBeInTheDocument()
  },
}

/** An organization admin sees Utilisateurs and a link to their own organization's admin page. */
export const AsOrganizationAdmin: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: MeService,
          useValue: fakeMe({
            isSuperAdmin: false,
            isOrganizationAdmin: true,
            organizationId: 'org-acme',
          }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() =>
      expect(canvas.getByRole('link', { name: /Utilisateurs/ })).toBeInTheDocument(),
    )
    // Org admins get their own "Administration" link too, scoped to their organization — not
    // the instance-wide one a super-admin sees.
    expect(canvas.getByRole('link', { name: 'Administration' }).getAttribute('href')).toContain(
      '/admin/organizations/org-acme',
    )
  },
}

/** Typing a query surfaces matching repositories, and selecting one navigates to it. */
export const SearchingForARepository: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const searchInput = await canvas.findByRole('combobox', {
      name: 'Rechercher un dépôt ou un utilisateur...',
    })
    await userEvent.type(searchInput, 'artiferris')
    await waitFor(() =>
      expect(canvas.getByRole('option', { name: 'artiferris-web' })).toBeInTheDocument(),
    )
    await userEvent.click(canvas.getByRole('option', { name: 'artiferris-web' }))
    await waitFor(() => expect(searchInput).toHaveValue(''))
  },
}

/**
 * From two characters on, packages and images from the readable repositories show up in their own
 * category; selecting one navigates to it.
 */
export const SearchingForAPackage: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const searchInput = await canvas.findByRole('combobox', {
      name: 'Rechercher un dépôt ou un utilisateur...',
    })
    await userEvent.type(searchInput, 'a')
    await waitFor(() =>
      expect(canvas.getByRole('option', { name: 'artiferris-web' })).toBeVisible(),
    )
    expect(canvas.queryByText('Paquets et images')).not.toBeInTheDocument()

    await userEvent.type(searchInput, 'p')
    await waitFor(() => expect(canvas.getByText('Paquets et images')).toBeVisible())
    expect(canvas.getByRole('option', { name: 'left-pad (npm)' })).toBeVisible()
    expect(canvas.getByRole('option', { name: 'api (Docker)' })).toBeVisible()

    await userEvent.click(canvas.getByRole('option', { name: 'left-pad (npm)' }))
    await waitFor(() => expect(searchInput).toHaveValue(''))
  },
}

/** A package that only exists in a proxy's cache says so. */
export const SearchingForACachedPackage: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: ReadableCatalogService,
          useValue: fakeReadableCatalog([proxiedEntry(), readableEntry({ name: 'lodash' })]),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const searchInput = await canvas.findByRole('combobox', {
      name: 'Rechercher un dépôt ou un utilisateur...',
    })
    await userEvent.type(searchInput, 'lodash')
    await waitFor(() =>
      expect(canvas.getByRole('option', { name: 'lodash (npm) — cache du proxy' })).toBeVisible(),
    )
    expect(canvas.getByRole('option', { name: 'lodash (npm)' })).toBeVisible()
  },
}

/** Without any matching package or image, the category is simply absent. */
export const SearchingWithoutMatchingPackages: Story = {
  decorators: [
    moduleMetadata({
      providers: [{ provide: ReadableCatalogService, useValue: fakeReadableCatalog([]) }],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const searchInput = await canvas.findByRole('combobox', {
      name: 'Rechercher un dépôt ou un utilisateur...',
    })
    await userEvent.type(searchInput, 'artiferris')
    await waitFor(() =>
      expect(canvas.getByRole('option', { name: 'artiferris-web' })).toBeVisible(),
    )
    await new Promise((resolve) => setTimeout(resolve, 500))
    expect(canvas.queryByText('Paquets et images')).not.toBeInTheDocument()
  },
}

/** A throttled or failing package search leaves the repositories and users in the bar untouched. */
export const PackageSearchFailing: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: ReadableCatalogService,
          useValue: {
            search: () => throwError(() => new HttpErrorResponse({ status: 429 })),
          },
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const searchInput = await canvas.findByRole('combobox', {
      name: 'Rechercher un dépôt ou un utilisateur...',
    })
    await userEvent.type(searchInput, 'artiferris')
    await waitFor(() =>
      expect(canvas.getByRole('option', { name: 'artiferris-web' })).toBeVisible(),
    )
    await new Promise((resolve) => setTimeout(resolve, 500))
    expect(canvas.queryByText('Paquets et images')).not.toBeInTheDocument()
    expect(canvas.getByRole('option', { name: 'artiferris-web' })).toBeVisible()
  },
}

/**
 * When the lists behind the search cannot be loaded, the bar says so instead of "Aucun résultat".
 */
export const SearchLoadFailed: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: RepositoriesService,
          useValue: { list: () => throwError(() => new Error('x')) },
        },
        { provide: UsersService, useValue: { list: () => throwError(() => new Error('x')) } },
        { provide: ReadableCatalogService, useValue: fakeReadableCatalog([]) },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const searchInput = await canvas.findByRole('combobox', {
      name: 'Rechercher un dépôt ou un utilisateur...',
    })
    await userEvent.type(searchInput, 'artiferris')
    expect(await canvas.findByText('La recherche a échoué')).toBeVisible()
  },
}

/** Opening the user menu shows account and logout actions. */
export const OpeningTheUserMenu: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('button', { name: /florian/ }))
    await waitFor(() =>
      expect(canvas.getByRole('menuitem', { name: 'Mon compte' })).toBeInTheDocument(),
    )
    expect(canvas.getByRole('menuitem', { name: 'Déconnexion' })).toBeInTheDocument()
  },
}

/** A failed /me load logs the user out instead of leaving the shell stuck loading. */
export const MeLoadFailed: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: MeService,
          useValue: { ...fakeMe({}), load: () => throwError(() => new Error('unauthorized')) },
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    await waitFor(() =>
      expect(within(canvasElement).queryByText('Chargement…')).not.toBeInTheDocument(),
    )
  },
}
