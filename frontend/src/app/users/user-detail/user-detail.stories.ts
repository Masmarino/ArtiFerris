import { signal } from '@angular/core'
import {
  applicationConfig,
  moduleMetadata,
  type Meta,
  type StoryObj,
} from '@storybook/angular-vite'
import { ActivatedRoute, convertToParamMap, provideRouter } from '@angular/router'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { of, throwError } from 'rxjs'
import { UserDetail } from './user-detail'
import { UsersService } from '../application/users.service'
import { PermissionsService } from '../../repositories/application/permissions.service'
import { RepositoriesService } from '../../repositories/application/repositories.service'
import { MeService } from '../../shell/application/me.service'
import type { UserSummary } from '../domain/user.entity'
import type { UserPermissionEntry } from '../../repositories/domain/permission.entity'
import type { RepositorySummary } from '../../repositories/domain/repository.entity'

const ACME_USER: UserSummary = {
  id: 'user-2',
  username: 'acme-user',
  is_super_admin: false,
  organization_id: 'org-acme',
  email: 'user@acme.example.com',
  invitation_pending: false,
}

const PENDING_USER: UserSummary = {
  ...ACME_USER,
  id: 'user-3',
  username: 'pending-user',
  email: null,
  invitation_pending: true,
}

const PERMISSIONS: UserPermissionEntry[] = [
  { repository_id: 'r1', repository_name: 'acme-npm', format: 'npm', role: 'write' },
  { repository_id: 'r2', repository_name: 'acme-docker', format: 'docker', role: 'read' },
]

const REPOSITORIES: RepositorySummary[] = [
  {
    id: 'r1',
    name: 'acme-npm',
    format: 'npm',
    repo_type: 'hosted',
    remote_url: null,
    remote_credentials_set: false,
    group_members: [],
    quota_bytes: null,
    retention_keep_last_n: null,
    my_role: 'admin',
    organization_id: 'org-acme',
  },
  {
    id: 'r2',
    name: 'acme-docker',
    format: 'docker',
    repo_type: 'hosted',
    remote_url: null,
    remote_credentials_set: false,
    group_members: [],
    quota_bytes: null,
    retention_keep_last_n: null,
    my_role: 'admin',
    organization_id: 'org-acme',
  },
]

function withRoute(userId: string) {
  return {
    provide: ActivatedRoute,
    useValue: {
      snapshot: { paramMap: convertToParamMap({ id: userId }) },
      paramMap: of(convertToParamMap({ id: userId })),
    },
  }
}
function fakeUsers(overrides: Partial<UsersService> = {}): Partial<UsersService> {
  return { get: () => of(ACME_USER), ...overrides }
}
function fakePermissions(overrides: Partial<PermissionsService> = {}): Partial<PermissionsService> {
  return { listForUser: () => of(PERMISSIONS), ...overrides }
}
function fakeRepositories(
  overrides: Partial<RepositoriesService> = {},
): Partial<RepositoriesService> {
  return { list: () => of(REPOSITORIES), ...overrides }
}
function fakeMe(isSuperAdmin: boolean): Partial<MeService> {
  return { isSuperAdmin: signal(isSuperAdmin) }
}

const meta: Meta<UserDetail> = {
  title: 'Users/UserDetail',
  component: UserDetail,
  decorators: [
    applicationConfig({ providers: [provideRouter([])] }),
    moduleMetadata({
      providers: [
        withRoute('user-2'),
        { provide: UsersService, useValue: fakeUsers() },
        { provide: PermissionsService, useValue: fakePermissions() },
        { provide: RepositoriesService, useValue: fakeRepositories() },
        { provide: MeService, useValue: fakeMe(true) },
      ],
    }),
  ],
}
export default meta

type Story = StoryObj<UserDetail>

/** As a super-admin: sees permissions, the grant form, and both promote/delete actions. */
export const Default: Story = {}

/** A plain organization admin can manage permissions but not the super-admin toggle. */
export const AsOrganizationAdmin: Story = {
  decorators: [moduleMetadata({ providers: [{ provide: MeService, useValue: fakeMe(false) }] })],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByText('acme-npm')).toBeInTheDocument())
    expect(
      canvas.queryByRole('button', { name: 'Promouvoir super-administrateur' }),
    ).not.toBeInTheDocument()
    expect(canvas.getByRole('button', { name: "Supprimer l'utilisateur" })).toBeInTheDocument()
  },
}

/** A pending invitation offers a resend action instead of showing permissions as settled. */
export const PendingInvitation: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        withRoute('user-3'),
        { provide: UsersService, useValue: fakeUsers({ get: () => of(PENDING_USER) }) },
        { provide: PermissionsService, useValue: fakePermissions({ listForUser: () => of([]) }) },
      ],
    }),
  ],
}

export const OpeningTheRoleEditor: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByText('acme-npm'))
    await waitFor(() =>
      expect(
        canvas.getByRole('heading', { name: "Modifier l'accès du dépôt acme-npm" }),
      ).toBeInTheDocument(),
    )
  },
}

export const LoadFailed: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: UsersService,
          useValue: fakeUsers({ get: () => throwError(() => new Error('not found')) }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    await waitFor(() => expect(within(canvasElement).getByRole('alert')).toBeInTheDocument())
  },
}
