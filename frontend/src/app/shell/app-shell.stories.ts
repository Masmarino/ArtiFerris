import {
  applicationConfig,
  moduleMetadata,
  type Meta,
  type StoryObj,
} from '@storybook/angular-vite'
import { provideRouter } from '@angular/router'
import { Component, signal } from '@angular/core'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { of, throwError } from 'rxjs'
import { AppShell } from './app-shell'
import { AuthService } from '../auth/application/auth.service'
import { MeService } from './application/me.service'
import { VersionService } from './application/version.service'
import { RepositoriesService } from '../repositories/application/repositories.service'
import { UsersService } from '../users/application/users.service'
import type { MeResponse } from './domain/me.entity'
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
  my_role: 'admin',
  organization_id: 'org-acme',
}

const USER: UserSummary = {
  id: 'u2',
  username: 'acme-writer',
  is_super_admin: false,
  organization_id: 'org-acme',
  email: null,
  invitation_pending: false,
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
