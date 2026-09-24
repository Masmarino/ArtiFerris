import { signal } from '@angular/core'
import {
  applicationConfig,
  moduleMetadata,
  type Meta,
  type StoryObj,
} from '@storybook/angular-vite'
import { provideRouter } from '@angular/router'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { NEVER, of, throwError } from 'rxjs'
import { RepositoriesList } from './repositories-list'
import { RepositoriesService } from '../application/repositories.service'
import { PersonalRepositoryService } from '../application/personal-repository.service'
import { OrganizationsService } from '../../admin/application/organizations.service'
import { MeService } from '../../shell/application/me.service'
import type { RepositorySummary } from '../domain/repository.entity'
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

const PUBLIC_REPO: RepositorySummary = {
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
  organization_id: 'org-public',
  owner_name: 'Public',
  owner_is_personal: false,
}
const ACME_REPO: RepositorySummary = {
  ...PUBLIC_REPO,
  id: 'r2',
  name: 'acme-docker',
  format: 'docker',
  organization_id: 'org-acme',
  owner_name: 'Acme Corp',
}

function fakeRepositories(
  overrides: Partial<RepositoriesService> = {},
): Partial<RepositoriesService> {
  return { list: () => of([PUBLIC_REPO]), ...overrides }
}
function fakeOrgs(overrides: Partial<OrganizationsService> = {}): Partial<OrganizationsService> {
  return { list: () => of([PUBLIC_ORG, ACME_ORG]), ...overrides }
}
function fakeMe(isSuperAdmin: boolean): Partial<MeService> {
  return { isSuperAdmin: signal(isSuperAdmin) }
}
function fakePersonalRepository(
  overrides: Partial<PersonalRepositoryService> = {},
): Partial<PersonalRepositoryService> {
  return { listMyProjects: () => of([{ ...PUBLIC_REPO, owner_is_personal: true }]), ...overrides }
}

const meta: Meta<RepositoriesList> = {
  title: 'Repositories/RepositoriesList',
  component: RepositoriesList,
  decorators: [
    applicationConfig({ providers: [provideRouter([])] }),
    moduleMetadata({
      providers: [
        { provide: RepositoriesService, useValue: fakeRepositories() },
        { provide: OrganizationsService, useValue: fakeOrgs() },
        { provide: MeService, useValue: fakeMe(true) },
        { provide: PersonalRepositoryService, useValue: fakePersonalRepository() },
      ],
    }),
  ],
}
export default meta

type Story = StoryObj<RepositoriesList>

/** As a super-admin: defaults to the public organization's repos, with the org filter shown. */
export const Default: Story = {}

export const FilteredToOneOrganization: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: RepositoriesService,
          useValue: fakeRepositories({ list: () => of([PUBLIC_REPO, ACME_REPO]) }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByText('artiferris-web')).toBeInTheDocument())
    await userEvent.click(canvas.getByRole('combobox', { name: 'Organisation' }))
    await userEvent.click(await canvas.findByRole('option', { name: 'Acme Corp' }))
    await waitFor(() => expect(canvas.getByText('acme-docker')).toBeInTheDocument())
    expect(canvas.queryByText('artiferris-web')).not.toBeInTheDocument()
  },
}

/** A plain organization admin: no filter, no organization column — just their own repos. */
export const AsOrganizationAdmin: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: RepositoriesService,
          useValue: fakeRepositories({ list: () => of([ACME_REPO]) }),
        },
        { provide: MeService, useValue: fakeMe(false) },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByText('acme-docker')).toBeInTheDocument())
    expect(canvas.queryByRole('combobox')).not.toBeInTheDocument()
  },
}

export const Empty: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        { provide: RepositoriesService, useValue: fakeRepositories({ list: () => of([]) }) },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByText('Aucun dépôt')).toBeInTheDocument()
    expect(canvas.queryByRole('table')).not.toBeInTheDocument()
    expect(canvas.getAllByRole('button', { name: 'Nouveau dépôt' })).toHaveLength(2)
  },
}

export const EmptyPersonalMode: Story = {
  args: { mode: 'personal' },
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: PersonalRepositoryService,
          useValue: fakePersonalRepository({ listMyProjects: () => of([]) }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    expect(await within(canvasElement).findByText('Aucun projet')).toBeInTheDocument()
  },
}

/** Repositories exist, but none belong to the organization picked in the filter. */
export const EmptyAfterFiltering: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: RepositoriesService,
          useValue: fakeRepositories({ list: () => of([PUBLIC_REPO]) }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByText('artiferris-web')).toBeInTheDocument())
    await userEvent.click(canvas.getByRole('combobox', { name: 'Organisation' }))
    await userEvent.click(await canvas.findByRole('option', { name: 'Acme Corp' }))
    expect(await canvas.findByText('Aucun résultat')).toBeInTheDocument()
    expect(canvas.queryByRole('table')).not.toBeInTheDocument()
  },
}

export const Loading: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        { provide: RepositoriesService, useValue: fakeRepositories({ list: () => NEVER }) },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByRole('status')).toHaveTextContent('Chargement…')
    expect(canvas.queryByRole('table')).not.toBeInTheDocument()
  },
}

export const LoadFailed: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: RepositoriesService,
          useValue: fakeRepositories({ list: () => throwError(() => new Error('down')) }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    await waitFor(() => expect(within(canvasElement).getByRole('alert')).toBeInTheDocument())
  },
}

export const CreateRepositoryModal: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('button', { name: 'Nouveau dépôt' }))
    await waitFor(() =>
      expect(canvas.getByRole('heading', { name: 'Nouveau dépôt' })).toBeInTheDocument(),
    )
  },
}

/** The caller's own personal namespace: no organization filter/column, even as a super-admin. */
export const PersonalMode: Story = {
  args: { mode: 'personal' },
  decorators: [
    moduleMetadata({
      providers: [{ provide: PersonalRepositoryService, useValue: fakePersonalRepository() }],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByText('artiferris-web')).toBeInTheDocument())
    expect(canvas.queryByRole('combobox')).not.toBeInTheDocument()
  },
}

export const CreateUserProjectModal: Story = {
  args: { mode: 'personal' },
  decorators: [
    moduleMetadata({
      providers: [{ provide: PersonalRepositoryService, useValue: fakePersonalRepository() }],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('button', { name: 'Nouveau projet' }))
    await waitFor(() =>
      expect(canvas.getByRole('heading', { name: 'Nouveau projet' })).toBeInTheDocument(),
    )
  },
}
