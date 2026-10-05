import {
  applicationConfig,
  moduleMetadata,
  type Meta,
  type StoryObj,
} from '@storybook/angular-vite'
import { Router, provideRouter } from '@angular/router'
import { Component, inject, provideAppInitializer, signal } from '@angular/core'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { HttpErrorResponse } from '@angular/common/http'
import { of, throwError } from 'rxjs'
import { AppShell } from './app-shell'
import { ADMIN_TRAIL } from './page-trail'
import { PageHeading } from '../shared/page-heading/page-heading'
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
  created_at: '2026-03-02T09:00:00Z',
  invitation_expires_at: null,
  mfa_enabled: false,
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

// Enough routes for navigateByUrl() to resolve instead of throwing NG04002; AppShell needs the real
// Router for routerLink and router-outlet.
@Component({ standalone: true, template: '' })
class DummyRoutedComponent {}

/** A page as every page under the shell starts: its own heading, the bar naming what is above it. */
@Component({
  standalone: true,
  imports: [PageHeading],
  template: `<div class="container">
    <app-page-heading />
    <p>The page's content.</p>
  </div>`,
})
class ExamplePage {}

/** Opens the quick search from the header's button and returns its field. */
async function openSearch(canvas: ReturnType<typeof within>): Promise<HTMLElement> {
  await userEvent.click(await canvas.findByRole('button', { name: 'Rechercher ou aller à…' }))
  return canvas.findByRole('combobox', { name: 'Recherche rapide' })
}

/** The options' own labels, without their descriptions. */
function optionLabels(canvasElement: HTMLElement): string[] {
  return Array.from(
    canvasElement.querySelectorAll<HTMLElement>('[role="option"] .gbt-cp__item-label'),
    (label) => label.textContent?.trim() ?? '',
  )
}

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
          {
            path: 'admin/export',
            component: ExamplePage,
            data: { trail: ADMIN_TRAIL, titleKey: 'nav.export' },
          },
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

export const Default: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByText('florian')).toBeInTheDocument())
    const administration = canvas.getByRole('button', { name: /Administration/ })
    expect(administration).toHaveAttribute('aria-expanded', 'false')
    // Utilisateurs is in Administration, as in FerrisGit: the dashboard first, the configuration last.
    await userEvent.click(administration)
    const links = await waitFor(() => {
      const found = Array.from(
        canvasElement.querySelectorAll<HTMLElement>('.gbt-app-shell-nav-group__panel a'),
      ).map((link) => link.textContent?.trim())
      expect(found.length).toBeGreaterThan(0)
      return found
    })
    expect(links).toEqual(['Tableau de bord', 'Utilisateurs', 'Organisations', 'Santé', 'Export'])
    expect(canvas.getByText('v0.4.6')).toBeInTheDocument()
  },
}

export const CollapsingTheSidebar: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByText('florian')).toBeInTheDocument())
    const toggle = canvas.getByRole('button', { name: 'Réduire le menu' })
    await userEvent.click(toggle)
    await waitFor(() =>
      expect(canvas.getByRole('button', { name: 'Déployer le menu' })).toBeInTheDocument(),
    )
    // The label stays for assistive technology; the collapsed rail only shows it as a flyout on hover or focus.
    expect(canvas.getByRole('link', { name: 'Dépôts' })).toBeInTheDocument()
    expect(canvas.getByText('Dépôts').getBoundingClientRect().width).toBe(0)
    expect(canvas.queryByAltText('ArtiFerris logo')).not.toBeInTheDocument()
    expect(canvas.getByAltText('ArtiFerris')).toHaveAttribute('src', '/Logo.png')
    expect(canvas.queryByText('v0.4.6')).not.toBeInTheDocument()
  },
}

let sessionLoadCalls = 0

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
    // Org admins get their users and their own organization's settings, in Administration.
    await userEvent.click(await canvas.findByRole('button', { name: /Administration/ }))
    expect(
      (await canvas.findByRole('link', { name: /Utilisateurs/ })).getAttribute('href'),
    ).toMatch(/\/users$/)
    expect(canvas.getByRole('link', { name: /Réglages/ }).getAttribute('href')).toContain(
      '/admin/organizations/org-acme',
    )
  },
}

export const QuickSearchOpen: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await openSearch(canvas)
    const goTo = await canvas.findByText('Aller à')
    // The palette fades in.
    await waitFor(() => expect(goTo).toBeVisible())
  },
}

/** A member sees no administration entry, whatever they type. */
export const QuickSearchAsPlainMember: Story = {
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
    const searchInput = await openSearch(canvas)
    await waitFor(() => expect(optionLabels(canvasElement)).toContain('Mon compte'))
    const administration = ['Tableau de bord', 'Utilisateurs', 'Organisations', 'Santé', 'Export']
    expect(optionLabels(canvasElement).filter((label) => administration.includes(label))).toEqual(
      [],
    )
    await userEvent.type(searchInput, 'acme')
    await waitFor(() => expect(optionLabels(canvasElement)).toContain('artiferris-web'))
    expect(canvas.queryByText('acme-writer')).not.toBeInTheDocument()
  },
}

export const SearchingForARepository: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const searchInput = await openSearch(canvas)
    await userEvent.type(searchInput, 'artiferris')
    await waitFor(() => expect(optionLabels(canvasElement)).toContain('artiferris-web'))
    await userEvent.click(canvas.getByText('artiferris-web'))
    await waitFor(() => expect(canvas.queryByRole('dialog')).not.toBeInTheDocument())
  },
}

export const SearchingForAPackage: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const searchInput = await openSearch(canvas)
    await userEvent.type(searchInput, 'a')
    await waitFor(() => expect(optionLabels(canvasElement)).toContain('artiferris-web'))
    expect(canvas.queryByText('Paquets et images')).not.toBeInTheDocument()

    await userEvent.type(searchInput, 'p')
    await waitFor(() => expect(canvas.getByText('Paquets et images')).toBeVisible())
    expect(optionLabels(canvasElement)).toEqual(expect.arrayContaining(['left-pad', 'api']))

    await userEvent.click(canvas.getByText('left-pad'))
    await waitFor(() => expect(canvas.queryByRole('dialog')).not.toBeInTheDocument())
  },
}

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
    const searchInput = await openSearch(canvas)
    await userEvent.type(searchInput, 'lodash')
    await waitFor(() => expect(canvas.getByText('npm, npmjs-proxy (cache du proxy)')).toBeVisible())
    expect(canvas.getByText('npm, test-npm')).toBeVisible()
  },
}

export const SearchingWithoutMatchingPackages: Story = {
  decorators: [
    moduleMetadata({
      providers: [{ provide: ReadableCatalogService, useValue: fakeReadableCatalog([]) }],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const searchInput = await openSearch(canvas)
    await userEvent.type(searchInput, 'artiferris')
    await waitFor(() => expect(optionLabels(canvasElement)).toContain('artiferris-web'))
    await new Promise((resolve) => setTimeout(resolve, 500))
    expect(canvas.queryByText('Paquets et images')).not.toBeInTheDocument()
  },
}

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
    const searchInput = await openSearch(canvas)
    await userEvent.type(searchInput, 'artiferris')
    await waitFor(() => expect(optionLabels(canvasElement)).toContain('artiferris-web'))
    await new Promise((resolve) => setTimeout(resolve, 500))
    expect(canvas.queryByText('Paquets et images')).not.toBeInTheDocument()
    expect(optionLabels(canvasElement)).toContain('artiferris-web')
  },
}

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
    const searchInput = await openSearch(canvas)
    await userEvent.type(searchInput, 'artiferris')
    expect(await canvas.findByText('La recherche a échoué')).toBeVisible()
  },
}

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

/** An admin page: the bar's breadcrumb names what is above it, the page names itself in its own h1. */
export const OnAPageUnderAdministration: Story = {
  decorators: [
    applicationConfig({
      providers: [provideAppInitializer(() => inject(Router).navigateByUrl('/admin/export'))],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const trail = await canvas.findByRole('navigation', { name: "Fil d'Ariane" })
    await waitFor(() =>
      expect(
        within(trail).getByRole('link', { name: 'Administration' }).getAttribute('href'),
      ).toMatch(/\/admin$/),
    )
    expect(await canvas.findByRole('heading', { level: 1, name: 'Export' })).toBeInTheDocument()
    expect(canvasElement.querySelectorAll('h1')).toHaveLength(1)
  },
}
