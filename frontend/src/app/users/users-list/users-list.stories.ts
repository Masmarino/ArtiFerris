import { signal } from '@angular/core'
import {
  applicationConfig,
  moduleMetadata,
  type Meta,
  type StoryObj,
} from '@storybook/angular-vite'
import { provideRouter } from '@angular/router'
import { expect, fn, userEvent, waitFor, within } from 'storybook/test'
import { NEVER, of, throwError } from 'rxjs'
import { UsersList } from './users-list'
import { UsersService } from '../application/users.service'
import { OrganizationsService } from '../../admin/application/organizations.service'
import { MeService } from '../../shell/application/me.service'
import { AuthService } from '../../auth/application/auth.service'
import type { UserSummary } from '../domain/user.entity'
import type { OrganizationSummary } from '../../admin/domain/organization.entity'
import { PageTitleService } from '../../shell/page-title.service'

const PUBLIC_ORG: OrganizationSummary = {
  id: 'org-public',
  slug: 'public',
  display_name: 'Public',
  is_public: true,
}
const ACME_ORG: OrganizationSummary = {
  id: 'org-acme',
  slug: 'acme',
  display_name: 'Acme Corp',
  is_public: false,
}

const now = Date.now()
const hoursAgo = (hours: number) => new Date(now - hours * 3_600_000).toISOString()
const daysAgo = (days: number) => hoursAgo(days * 24)
const inHours = (hours: number) => hoursAgo(-hours)

function user(overrides: Partial<UserSummary> & Pick<UserSummary, 'id' | 'username'>): UserSummary {
  return {
    is_super_admin: false,
    organization_id: 'org-public',
    email: `${overrides.username}@example.com`,
    invitation_pending: false,
    created_at: daysAgo(30),
    invitation_expires_at: null,
    mfa_enabled: true,
    ...overrides,
  }
}

const USERS: UserSummary[] = [
  user({ id: 'u1', username: 'florian', is_super_admin: true, created_at: daysAgo(120) }),
  user({ id: 'u2', username: 'bob', created_at: daysAgo(45) }),
  user({ id: 'u3', username: 'carol', mfa_enabled: false, created_at: daysAgo(6) }),
  user({
    id: 'u4',
    username: 'dave',
    invitation_pending: true,
    mfa_enabled: false,
    invitation_expires_at: inHours(19),
    created_at: hoursAgo(5),
  }),
  user({
    id: 'u5',
    username: 'erin',
    invitation_pending: true,
    mfa_enabled: false,
    invitation_expires_at: daysAgo(2),
    created_at: daysAgo(4),
  }),
  user({
    id: 'u6',
    username: 'acme-admin',
    organization_id: 'org-acme',
    email: 'admin@acme.example.com',
    created_at: daysAgo(60),
  }),
]

function fakeUsers(overrides: Partial<UsersService> = {}): Partial<UsersService> {
  return { list: () => of(USERS), ...overrides }
}
function fakeOrgs(overrides: Partial<OrganizationsService> = {}): Partial<OrganizationsService> {
  return { list: () => of([PUBLIC_ORG, ACME_ORG]), ...overrides }
}
function fakeMe(isSuperAdmin: boolean): Partial<MeService> {
  return { isSuperAdmin: signal(isSuperAdmin), username: signal('florian') }
}

const meta: Meta<UsersList> = {
  title: 'Users/UsersList',
  component: UsersList,
  decorators: [
    applicationConfig({ providers: [provideRouter([])] }),
    moduleMetadata({
      providers: [
        // The shell sets the title from the route.
        { provide: PageTitleService, useValue: { title: signal('Utilisateurs') } },
        { provide: AuthService, useValue: { logout: fn() } },
        { provide: UsersService, useValue: fakeUsers() },
        { provide: OrganizationsService, useValue: fakeOrgs() },
        { provide: MeService, useValue: fakeMe(true) },
      ],
    }),
  ],
}
export default meta

type Story = StoryObj<UsersList>

export const Default: Story = {}

export const PendingInvitations: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('radio', { name: /Invitations en attente/ }))
    await waitFor(() => expect(canvas.queryByText('bob')).not.toBeInTheDocument())
    expect(canvas.getByText('dave@example.com')).toBeInTheDocument()
    expect(canvas.getByText('Invitation expirée')).toBeInTheDocument()
  },
}

export const RowMenu: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(
      await canvas.findByRole('button', { name: 'Actions pour dave@example.com' }),
    )
    expect(
      await within(canvasElement.ownerDocument.body).findByRole('menuitem', {
        name: "Renvoyer l'invitation",
      }),
    ).toBeInTheDocument()
  },
}

export const ResetMfaMenu: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('button', { name: 'Actions pour bob' }))
    expect(
      await within(canvasElement.ownerDocument.body).findByRole('menuitem', {
        name: 'Réinitialiser la double authentification',
      }),
    ).toBeInTheDocument()
    expect(
      within(canvasElement.ownerDocument.body).getByRole('menuitem', {
        name: 'Réinitialiser le mot de passe',
      }),
    ).toBeInTheDocument()
  },
}

export const FilteredToOneOrganization: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByText('florian')).toBeInTheDocument())
    await userEvent.click(canvas.getByRole('combobox', { name: 'Organisation' }))
    await userEvent.click(await canvas.findByRole('option', { name: 'Acme Corp' }))
    await waitFor(() => expect(canvas.getByText('acme-admin')).toBeInTheDocument())
    expect(canvas.queryByText('florian')).not.toBeInTheDocument()
  },
}

export const AsOrganizationAdmin: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: UsersService,
          useValue: fakeUsers({
            list: () => of(USERS.filter((u) => u.organization_id === 'org-acme')),
          }),
        },
        { provide: MeService, useValue: fakeMe(false) },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByText('acme-admin')).toBeInTheDocument()
    expect(canvas.queryByRole('button', { name: 'Inviter un utilisateur' })).not.toBeInTheDocument()
    expect(canvas.queryByRole('combobox', { name: 'Organisation' })).not.toBeInTheDocument()
  },
}

export const Loading: Story = {
  decorators: [
    moduleMetadata({
      providers: [{ provide: UsersService, useValue: fakeUsers({ list: () => NEVER }) }],
    }),
  ],
  play: async ({ canvasElement }) => {
    expect(await within(canvasElement).findByRole('status')).toHaveTextContent(
      'Chargement des utilisateurs…',
    )
  },
}

export const Empty: Story = {
  decorators: [
    moduleMetadata({
      providers: [{ provide: UsersService, useValue: fakeUsers({ list: () => of([]) }) }],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByText('Aucun utilisateur')).toBeInTheDocument()
    expect(canvas.getAllByRole('button', { name: 'Inviter un utilisateur' })).toHaveLength(2)
  },
}

export const EmptyForSelectedOrganization: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: UsersService,
          useValue: fakeUsers({
            list: () => of(USERS.filter((u) => u.organization_id !== 'org-public')),
          }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(
      await canvas.findByText('Aucun utilisateur dans cette organisation.'),
    ).toBeInTheDocument()
  },
}

export const LoadFailed: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: UsersService,
          useValue: fakeUsers({ list: () => throwError(() => new Error('network error')) }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    await waitFor(() => expect(within(canvasElement).getByRole('alert')).toBeInTheDocument())
  },
}

export const CreateUserModal: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('button', { name: 'Inviter un utilisateur' }))
    await waitFor(() =>
      expect(canvas.getByRole('heading', { name: 'Inviter un utilisateur' })).toBeInTheDocument(),
    )
  },
}
