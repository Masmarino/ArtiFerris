import { moduleMetadata, type Meta, type StoryObj } from '@storybook/angular-vite'
import { ActivatedRoute, Router, convertToParamMap } from '@angular/router'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { of, throwError } from 'rxjs'
import { RepositoryDetail } from './repository-detail'
import { RepositoriesService } from '../application/repositories.service'
import { PermissionsService } from '../application/permissions.service'
import type { RepositorySummary, RepositoryPackages } from '../domain/repository.entity'
import type { PermissionEntry, UserLookup } from '../domain/permission.entity'

const HOSTED_REPO: RepositorySummary = {
  id: 'repo-1',
  name: 'acme-npm',
  format: 'npm',
  repo_type: 'hosted',
  remote_url: null,
  remote_credentials_set: false,
  group_members: [],
  quota_bytes: 500 * 1024 * 1024,
  retention_keep_last_n: 10,
  my_role: 'admin',
  organization_id: 'org-acme',
}

const PROXY_REPO: RepositorySummary = {
  ...HOSTED_REPO,
  id: 'repo-proxy',
  name: 'npm-mirror',
  repo_type: 'proxy',
  remote_url: 'https://registry.npmjs.org',
}

const GROUP_REPO: RepositorySummary = {
  ...HOSTED_REPO,
  id: 'repo-group',
  name: 'acme-group',
  repo_type: 'group',
  group_members: ['repo-1', 'repo-2'],
}

const OTHER_REPOS: RepositorySummary[] = [
  HOSTED_REPO,
  { ...HOSTED_REPO, id: 'repo-2', name: 'acme-docker', format: 'docker' },
]

const PERMISSIONS: PermissionEntry[] = [
  { user_id: 'u1', username: 'acme-writer', role: 'write' },
  { user_id: 'u2', username: 'acme-reader', role: 'read' },
]

const NPM_PACKAGES: RepositoryPackages = {
  format: 'npm',
  packages: [
    {
      name: '@acme/ui',
      versions: [
        {
          version: '1.2.0',
          published_at: '2026-09-01T00:00:00Z',
          size_bytes: 42_000,
          deprecated: false,
        },
      ],
      vulnerability_summary: { critical: 0, high: 1, medium: 2, low: 0 },
    },
  ],
}

function withRoute(id: string) {
  return {
    provide: ActivatedRoute,
    useValue: {
      snapshot: { paramMap: convertToParamMap({ id }) },
      paramMap: of(convertToParamMap({ id })),
    },
  }
}
function fakeRepositories(
  overrides: Partial<RepositoriesService> = {},
): Partial<RepositoriesService> {
  return {
    get: () => of(HOSTED_REPO),
    list: () => of(OTHER_REPOS),
    packages: () => of(NPM_PACKAGES),
    setQuota: () => of(undefined),
    setRetentionPolicy: () => of(undefined),
    rename: () => of(undefined),
    ...overrides,
  }
}
function fakePermissions(overrides: Partial<PermissionsService> = {}): Partial<PermissionsService> {
  return {
    list: () => of(PERMISSIONS),
    searchUsers: () => of<UserLookup[]>([{ id: 'u3', username: 'new-writer' }]),
    grant: () => of(undefined),
    ...overrides,
  }
}

const meta: Meta<RepositoryDetail> = {
  title: 'Repositories/RepositoryDetail',
  component: RepositoryDetail,
  decorators: [
    moduleMetadata({
      providers: [
        withRoute('repo-1'),
        // A real Router (via provideRouter) tries to match the Storybook iframe's own URL
        // against the (empty) route table and errors — this component only ever calls
        // .navigate(), so a plain stub sidesteps that entirely.
        { provide: Router, useValue: { navigate: () => Promise.resolve(true) } },
        { provide: RepositoriesService, useValue: fakeRepositories() },
        { provide: PermissionsService, useValue: fakePermissions() },
      ],
    }),
  ],
}
export default meta

type Story = StoryObj<RepositoryDetail>

/** A hosted repo, as its admin: rename, usage instructions, and every management tab. */
export const Default: Story = {}

/** A proxy repo shows the mirrored remote URL on its overview tab. */
export const ProxyRepository: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        { provide: RepositoriesService, useValue: fakeRepositories({ get: () => of(PROXY_REPO) }) },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    await waitFor(() =>
      expect(within(canvasElement).getByText('https://registry.npmjs.org')).toBeInTheDocument(),
    )
  },
}

/** A group repo gets its own "Dépôts membres" tab, resolving member ids to names. */
export const GroupRepository: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        { provide: RepositoriesService, useValue: fakeRepositories({ get: () => of(GROUP_REPO) }) },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('tab', { name: 'Dépôts membres' }))
    await waitFor(() => expect(canvas.getByText('acme-docker')).toBeInTheDocument())
  },
}

/** A non-admin never sees the permissions/settings tabs or the rename control. */
export const AsNonAdmin: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: RepositoriesService,
          useValue: fakeRepositories({ get: () => of({ ...HOSTED_REPO, my_role: 'read' }) }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    // The repo name only ever renders as visible text inside the (admin-only) rename input —
    // wait on something every viewer sees instead: the always-present "Aperçu" tab.
    await waitFor(() => expect(canvas.getByRole('tab', { name: 'Aperçu' })).toBeInTheDocument())
    expect(canvas.queryByRole('tab', { name: "Droits d'accès" })).not.toBeInTheDocument()
    expect(canvas.queryByRole('tab', { name: 'Paramètres' })).not.toBeInTheDocument()
    expect(canvas.queryByLabelText('Nom')).not.toBeInTheDocument()
  },
}

export const PackagesTab: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('tab', { name: 'Packages' }))
    await waitFor(() => expect(canvas.getByText('@acme/ui')).toBeInTheDocument())
  },
}

export const SavingQuota: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('tab', { name: 'Paramètres' }))
    await waitFor(() =>
      expect(canvas.getAllByRole('button', { name: 'Enregistrer' }).length).toBeGreaterThan(0),
    )
    // The quota card comes first in the "Paramètres" tab, so its "Enregistrer" button is [0].
    await userEvent.click(canvas.getAllByRole('button', { name: 'Enregistrer' })[0])
    await waitFor(() => expect(canvas.getByText('Quota enregistré.')).toBeInTheDocument())
  },
}

export const InvalidQuota: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('tab', { name: 'Paramètres' }))
    const quotaInput = await canvas.findByLabelText('Quota (Mo, vide = illimité)')
    await userEvent.clear(quotaInput)
    await userEvent.type(quotaInput, '-5')
    const saveButtons = canvas.getAllByRole('button', { name: 'Enregistrer' })
    await userEvent.click(saveButtons[0])
    await waitFor(() =>
      expect(
        canvas.getByText('Doit être un nombre positif (ou vide pour illimité).'),
      ).toBeInTheDocument(),
    )
  },
}

/** Typing a username in the grant form surfaces search results to pick from. */
export const SearchingForAUserToGrant: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('tab', { name: "Droits d'accès" }))
    // Under this addon's test runner this story's component tree can end up mounted twice in
    // the same canvas (a render-check pass plus the interaction pass) — the most recently
    // mounted copy is the live one the play function actually drives.
    const usernameInputs = await waitFor(() => {
      const inputs = canvas.getAllByPlaceholderText("Nom d'utilisateur")
      expect(inputs.length).toBeGreaterThan(0)
      return inputs
    })
    const usernameInput = usernameInputs[usernameInputs.length - 1]
    await userEvent.type(usernameInput, 'new')
    await waitFor(() => expect(canvas.getByText('new-writer')).toBeInTheDocument())
  },
}

export const OpeningTheRoleEditor: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('tab', { name: "Droits d'accès" }))
    await userEvent.click(await canvas.findByText('acme-writer'))
    await waitFor(() =>
      expect(
        canvas.getByRole('heading', { name: "Modifier l'accès de acme-writer" }),
      ).toBeInTheDocument(),
    )
  },
}

export const LoadFailed: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: RepositoriesService,
          useValue: fakeRepositories({ get: () => throwError(() => new Error('not found')) }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    await waitFor(() =>
      expect(within(canvasElement).getByRole('alert')).toHaveTextContent(
        'Échec du chargement du dépôt.',
      ),
    )
  },
}
