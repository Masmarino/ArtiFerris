import { signal } from '@angular/core'
import { moduleMetadata, type Meta, type StoryObj } from '@storybook/angular-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { NEVER, of, throwError } from 'rxjs'
import { UsersList } from './users-list'
import { UsersService } from '../application/users.service'
import { OrganizationsService } from '../../admin/application/organizations.service'
import { MeService } from '../../shell/application/me.service'
import type { UserSummary } from '../domain/user.entity'
import type { OrganizationSummary } from '../../admin/domain/organization.entity'

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

const USERS: UserSummary[] = [
  {
    id: 'u1',
    username: 'florian',
    is_super_admin: true,
    organization_id: 'org-public',
    email: 'florian@example.com',
    invitation_pending: false,
  },
  {
    id: 'u2',
    username: 'acme-admin',
    is_super_admin: false,
    organization_id: 'org-acme',
    email: 'admin@acme.example.com',
    invitation_pending: false,
  },
  {
    id: 'u3',
    username: 'new-hire',
    is_super_admin: false,
    organization_id: 'org-acme',
    email: null,
    invitation_pending: true,
  },
]

function fakeUsers(overrides: Partial<UsersService> = {}): Partial<UsersService> {
  return { list: () => of(USERS), ...overrides }
}
function fakeOrgs(overrides: Partial<OrganizationsService> = {}): Partial<OrganizationsService> {
  return { list: () => of([PUBLIC_ORG, ACME_ORG]), ...overrides }
}
function fakeMe(isSuperAdmin: boolean): Partial<MeService> {
  return { isSuperAdmin: signal(isSuperAdmin) }
}

const meta: Meta<UsersList> = {
  title: 'Users/UsersList',
  component: UsersList,
  decorators: [
    moduleMetadata({
      providers: [
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
}

export const Loading: Story = {
  decorators: [
    moduleMetadata({
      providers: [{ provide: UsersService, useValue: fakeUsers({ list: () => NEVER }) }],
    }),
  ],
  play: async ({ canvasElement }) => {
    expect(await within(canvasElement).findByRole('status')).toHaveTextContent('Chargement…')
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
    expect(canvas.getAllByRole('button', { name: 'Nouvel utilisateur' })).toHaveLength(2)
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
    expect(await canvas.findByText('Aucun résultat')).toBeInTheDocument()
    expect(canvas.queryByRole('table')).not.toBeInTheDocument()
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
    await userEvent.click(await canvas.findByRole('button', { name: 'Nouvel utilisateur' }))
    await waitFor(() =>
      expect(canvas.getByRole('heading', { name: 'Nouvel utilisateur' })).toBeInTheDocument(),
    )
  },
}
