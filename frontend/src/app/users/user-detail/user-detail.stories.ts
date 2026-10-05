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
import { PageTitleService } from '../../shell/page-title.service'
import { HttpErrorResponse } from '@angular/common/http'
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
  created_at: '2026-03-02T09:00:00Z',
  invitation_expires_at: null,
  mfa_enabled: true,
}

const PENDING_USER: UserSummary = {
  ...ACME_USER,
  id: 'user-3',
  username: 'pending-user',
  email: null,
  invitation_pending: true,
  invitation_expires_at: '2026-10-07T08:00:00Z',
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
    is_public: false,
    my_role: 'admin',
    organization_id: 'org-acme',
    owner_name: 'Acme Corp',
    owner_is_personal: false,
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
    is_public: false,
    my_role: 'admin',
    organization_id: 'org-acme',
    owner_name: 'Acme Corp',
    owner_is_personal: false,
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
  return { isSuperAdmin: signal(isSuperAdmin), username: signal('florian') }
}

const meta: Meta<UserDetail> = {
  title: 'Users/UserDetail',
  component: UserDetail,
  decorators: [
    applicationConfig({ providers: [provideRouter([])] }),
    moduleMetadata({
      providers: [
        withRoute('user-2'),
        // The shell's title, which the page sets once the user is in.
        { provide: PageTitleService, useValue: { title: signal('') } },
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

export const Default: Story = {}

export const AsOrganizationAdmin: Story = {
  decorators: [moduleMetadata({ providers: [{ provide: MeService, useValue: fakeMe(false) }] })],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByText('acme-npm')).toBeInTheDocument())
    // An organization admin resets second factors but grants no super-administrator rights.
    await userEvent.click(canvas.getByRole('button', { name: 'Actions' }))
    const menu = within(canvasElement.ownerDocument.body)
    expect(
      await menu.findByRole('menuitem', { name: 'Réinitialiser la double authentification' }),
    ).toBeInTheDocument()
    expect(
      menu.queryByRole('menuitem', { name: 'Nommer super-administrateur' }),
    ).not.toBeInTheDocument()
    expect(canvas.getByRole('button', { name: "Supprimer l'utilisateur" })).toBeInTheDocument()
  },
}

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

export const NoPermissions: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        { provide: PermissionsService, useValue: fakePermissions({ listForUser: () => of([]) }) },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    expect(await within(canvasElement).findByText("Aucun droit d'accès")).toBeInTheDocument()
  },
}

export const OpeningTheRoleEditor: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('button', { name: 'Actions pour acme-npm' }))
    await userEvent.click(
      await within(canvasElement.ownerDocument.body).findByRole('menuitem', {
        name: 'Changer le rôle',
      }),
    )
    await waitFor(() =>
      expect(
        canvas.getByRole('heading', { name: "Modifier l'accès du dépôt acme-npm" }),
      ).toBeInTheDocument(),
    )
  },
}

export const ActionsMenu: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('button', { name: 'Actions' }))
    expect(
      await within(canvasElement.ownerDocument.body).findByRole('menuitem', {
        name: 'Nommer super-administrateur',
      }),
    ).toBeInTheDocument()
  },
}

export const NotFound: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: UsersService,
          useValue: fakeUsers({
            get: () => throwError(() => new HttpErrorResponse({ status: 404 })),
          }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    expect(await within(canvasElement).findByText('Utilisateur introuvable')).toBeVisible()
  },
}

export const OwnAccount: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: UsersService,
          useValue: fakeUsers({ get: () => of({ ...ACME_USER, username: 'florian' }) }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    expect(await within(canvasElement).findByText("C'est votre propre compte")).toBeVisible()
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
