import {
  applicationConfig,
  moduleMetadata,
  type Meta,
  type StoryObj,
} from '@storybook/angular-vite'
import { ActivatedRoute, convertToParamMap, provideRouter } from '@angular/router'
import { expect, waitFor, within } from 'storybook/test'
import { NEVER, of, throwError } from 'rxjs'
import { HttpErrorResponse } from '@angular/common/http'
import { AuthService } from '../../auth/application/auth.service'
import { NO_SUGGESTIONS } from '../catalog/testing/no-suggestions'
import { PublicRepositoryPage } from './public-repository-page'
import { RepositoriesService } from '../../repositories/application/repositories.service'
import type {
  RepositoryPackages,
  RepositorySummary,
} from '../../repositories/domain/repository.entity'

const REPO: RepositorySummary = {
  id: 'repo-1',
  name: 'ui-kit',
  format: 'npm',
  repo_type: 'hosted',
  remote_url: null,
  remote_credentials_set: false,
  group_members: [],
  quota_bytes: null,
  retention_keep_last_n: null,
  is_public: true,
  my_role: null,
  organization_id: 'org-1',
  owner_name: 'alice',
  owner_is_personal: true,
}

const PACKAGES: RepositoryPackages = {
  format: 'npm',
  packages: [
    {
      name: '@alice/button',
      versions: [
        {
          version: '2.1.0',
          published_at: '2026-09-01T00:00:00Z',
          size_bytes: 12_000,
          deprecated: false,
        },
      ],
      truncated: false,
      vulnerability_summary: { critical: 0, high: 0, medium: 0, low: 0 },
    },
    {
      name: '@alice/tooltip',
      versions: [
        {
          version: '1.0.3',
          published_at: '2026-08-15T00:00:00Z',
          size_bytes: 4_000,
          deprecated: false,
        },
      ],
      truncated: false,
      vulnerability_summary: { critical: 0, high: 1, medium: 0, low: 0 },
    },
  ],
}

function withRoute(username: string, repoName: string) {
  const params = { username, repoName }
  return {
    provide: ActivatedRoute,
    useValue: { paramMap: of(convertToParamMap(params)) },
  }
}
function withOrgRoute(repoName: string) {
  const params = { slug: 'acme', repoName }
  return {
    provide: ActivatedRoute,
    useValue: { paramMap: of(convertToParamMap(params)) },
  }
}
function fakeRepositories(
  overrides: Partial<RepositoriesService> = {},
): Partial<RepositoriesService> {
  return {
    getByOwner: () => of(REPO),
    getByOrg: () => of({ ...REPO, owner_name: 'Acme Corp', owner_is_personal: false }),
    packages: () => of(PACKAGES),
    ...overrides,
  }
}

const meta: Meta<PublicRepositoryPage> = {
  title: 'Public/PublicRepositoryPage',
  component: PublicRepositoryPage,
  decorators: [
    applicationConfig({
      providers: [
        provideRouter([]),
        NO_SUGGESTIONS,
        { provide: AuthService, useValue: { isAuthenticated: () => false } },
      ],
    }),
    moduleMetadata({
      providers: [
        withRoute('@alice', 'ui-kit'),
        { provide: RepositoriesService, useValue: fakeRepositories() },
      ],
    }),
  ],
}
export default meta

type Story = StoryObj<PublicRepositoryPage>

/** A public hosted npm repository, with its usage instructions and package list. */
export const Default: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByText('@alice/button')).toBeInTheDocument())
  },
}

/** A private or unknown repository shows a not-found message, never which of the two it was. */
export const NotFound: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: RepositoriesService,
          useValue: fakeRepositories({
            getByOwner: () => throwError(() => new HttpErrorResponse({ status: 404 })),
          }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    await waitFor(() =>
      expect(within(canvasElement).getByRole('alert')).toHaveTextContent('Dépôt introuvable.'),
    )
  },
}

export const Loading: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: RepositoriesService,
          useValue: fakeRepositories({ getByOwner: () => NEVER }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByRole('status')).toHaveTextContent('Chargement…')
    expect(canvas.queryByRole('alert')).not.toBeInTheDocument()
  },
}

/** A server error is reported as a failure, not as "not found". */
export const LoadFailed: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: RepositoriesService,
          useValue: fakeRepositories({
            getByOwner: () => throwError(() => new HttpErrorResponse({ status: 500 })),
          }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() =>
      expect(canvas.getByRole('alert')).toHaveTextContent('Échec du chargement du dépôt.'),
    )
    expect(canvas.queryByText('Dépôt introuvable.')).not.toBeInTheDocument()
  },
}

/** Reached at /o/:slug/:repoName: resolved by organization slug, package links stay under /o. */
export const OrganizationRepository: Story = {
  decorators: [moduleMetadata({ providers: [withOrgRoute('ui-kit')] })],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByRole('heading', { name: 'ui-kit' })).toBeInTheDocument()
    expect(await canvas.findByRole('link', { name: /@alice\/button/ })).toHaveAttribute(
      'href',
      expect.stringMatching(/\/o\/acme\/ui-kit\/packages\/npm\/@alice%2Fbutton$/),
    )
  },
}

export const OrganizationRepositoryNotFound: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        withOrgRoute('ghost'),
        {
          provide: RepositoriesService,
          useValue: fakeRepositories({
            getByOrg: () => throwError(() => new HttpErrorResponse({ status: 404 })),
          }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    await waitFor(() =>
      expect(within(canvasElement).getByRole('alert')).toHaveTextContent('Dépôt introuvable.'),
    )
  },
}
