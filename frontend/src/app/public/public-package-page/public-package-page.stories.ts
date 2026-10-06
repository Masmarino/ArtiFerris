import {
  applicationConfig,
  moduleMetadata,
  type Meta,
  type StoryObj,
} from '@storybook/angular-vite'
import { ActivatedRoute, convertToParamMap, provideRouter } from '@angular/router'
import { HttpErrorResponse } from '@angular/common/http'
import { expect, waitFor, within } from 'storybook/test'
import { NEVER, of, throwError } from 'rxjs'
import { AuthService } from '../../auth/application/auth.service'
import { NO_SUGGESTIONS } from '../catalog/testing/no-suggestions'
import { OVERFLOW_README_HTML, RICH_README_HTML } from '../../shared/readme-view/readme.fixtures'
import { PublicPackagePage } from './public-package-page'
import { RepositoriesService } from '../../repositories/application/repositories.service'
import type {
  DockerImageDetails,
  NpmPackageDetails,
  RepositoryFormat,
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
  is_public: true,
  my_role: null,
  owner_name: 'alice',
  owner_is_personal: true,
}

const NPM_DETAILS: NpmPackageDetails = {
  name: '@alice/button',
  versions: [
    {
      version: '2.1.0',
      published_at: '2026-09-01T00:00:00Z',
      size_bytes: 12_000,
      deprecated: false,
      deprecated_message: null,
      shasum: 'abc',
    },
    {
      version: '2.0.0',
      published_at: '2026-08-01T00:00:00Z',
      size_bytes: 11_000,
      deprecated: false,
      deprecated_message: null,
      shasum: 'def',
    },
  ],
  dist_tags: [
    { tag: 'latest', version: '2.1.0' },
    { tag: 'next', version: '2.1.0' },
  ],
  truncated: false,
  readme_html: RICH_README_HTML,
  registry_url: 'http://localhost:4200/npm/u/alice/ui-kit/',
  downloads_7d: 1234,
}

const DOCKER_DETAILS: DockerImageDetails = {
  image_name: 'web',
  image_reference: 'localhost:4200/u/alice/ui-kit/web',
  downloads_7d: 56,
  truncated: false,
  tags: [
    {
      tag: 'latest',
      digest: 'sha256:abcdef0123456789',
      media_type: 'application/vnd.oci.image.manifest.v1+json',
      created_at: '2026-09-01T00:00:00Z',
      size_bytes: 48_234_496,
    },
    {
      tag: 'v1.0.0',
      digest: 'sha256:0123456789abcdef',
      media_type: 'application/vnd.oci.image.manifest.v1+json',
      created_at: '2026-08-01T00:00:00Z',
      size_bytes: null,
    },
  ],
}

function withRoute(format: RepositoryFormat, name: string) {
  const params = { username: '@alice', repoName: 'ui-kit', format, name }
  return {
    provide: ActivatedRoute,
    useValue: { paramMap: of(convertToParamMap(params)) },
  }
}
function withOrgRoute(format: RepositoryFormat, name: string) {
  const params = { slug: 'acme', repoName: 'ui-kit', format, name }
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
    npmPackageDetails: () => of(NPM_DETAILS),
    dockerImageDetails: () => of(DOCKER_DETAILS),
    npmPackageAudit: () => of([]),
    getDependencyAudit: () => of(null),
    getDockerImageScan: () => of(null),
    ...overrides,
  }
}
function withRepositories(overrides: Partial<RepositoriesService>) {
  return moduleMetadata({
    providers: [{ provide: RepositoriesService, useValue: fakeRepositories(overrides) }],
  })
}

const meta: Meta<PublicPackagePage> = {
  title: 'Public/PublicPackagePage',
  component: PublicPackagePage,
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
        withRoute('npm', '@alice/button'),
        { provide: RepositoriesService, useValue: fakeRepositories() },
      ],
    }),
  ],
}
export default meta

type Story = StoryObj<PublicPackagePage>

export const NpmPackage: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByRole('heading', { name: '@alice/button' })).toBeInTheDocument()
    expect(canvasElement.querySelector('app-copyable-command pre')).toHaveTextContent(
      'npm install @alice/button --registry http://localhost:4200/npm/u/alice/ui-kit/',
    )
    expect(canvas.getByText(/^1\s234 téléchargements cette semaine$/)).toBeInTheDocument()
    expect(canvas.getByRole('heading', { name: 'README', level: 2 })).toBeInTheDocument()
    expect(canvas.getByRole('heading', { name: 'Usage' })).toBeInTheDocument()
    expect(canvas.getByText('latest → 2.1.0')).toBeInTheDocument()
    expect(canvas.getByText('next → 2.1.0')).toBeInTheDocument()
    expect(canvas.getByRole('cell', { name: '2.1.0' })).toBeInTheDocument()
    expect(canvas.getByRole('cell', { name: '2.0.0' })).toBeInTheDocument()
    expect(canvas.getByRole('link', { name: '← Retour au dépôt' })).toHaveAttribute(
      'href',
      expect.stringMatching(/\/@alice\/ui-kit$/),
    )
  },
}

export const NpmPackageWithoutDownloads: Story = {
  decorators: [
    withRepositories({ npmPackageDetails: () => of({ ...NPM_DETAILS, downloads_7d: 0 }) }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByRole('heading', { name: '@alice/button' })).toBeInTheDocument()
    expect(canvas.queryByText(/téléchargement/)).toBeNull()
  },
}

export const NpmPackageWithTruncatedVersions: Story = {
  decorators: [
    withRepositories({ npmPackageDetails: () => of({ ...NPM_DETAILS, truncated: true }) }),
  ],
  play: async ({ canvasElement }) => {
    expect(
      await within(canvasElement).findByText(
        'Seules les 200 versions les plus récentes sont affichées.',
      ),
    ).toBeInTheDocument()
  },
}

export const DockerImageWithTruncatedTags: Story = {
  decorators: [
    moduleMetadata({ providers: [withRoute('docker', 'web')] }),
    withRepositories({ dockerImageDetails: () => of({ ...DOCKER_DETAILS, truncated: true }) }),
  ],
  play: async ({ canvasElement }) => {
    expect(
      await within(canvasElement).findByText('Seuls les 100 tags les plus récents sont affichés.'),
    ).toBeInTheDocument()
  },
}

export const NpmPackageWithoutReadme: Story = {
  decorators: [
    withRepositories({ npmPackageDetails: () => of({ ...NPM_DETAILS, readme_html: null }) }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByText('Aucun README')).toBeInTheDocument()
    expect(canvas.getByRole('cell', { name: '2.1.0' })).toBeInTheDocument()
  },
}

export const NpmPackageWithOverflowingReadme: Story = {
  decorators: [
    withRepositories({
      npmPackageDetails: () => of({ ...NPM_DETAILS, readme_html: OVERFLOW_README_HTML }),
    }),
  ],
  play: async ({ canvasElement }) => {
    await within(canvasElement).findByRole('heading', { name: 'README', level: 2 })
    const card = canvasElement.querySelector<HTMLElement>('.readme-view')!
    expect(card.scrollWidth).toBeLessThanOrEqual(card.clientWidth)
  },
}

export const NpmPackageWithoutDistTags: Story = {
  decorators: [
    withRepositories({ npmPackageDetails: () => of({ ...NPM_DETAILS, dist_tags: [] }) }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByRole('cell', { name: '2.1.0' })).toBeInTheDocument()
    expect(canvas.queryByText(/→/)).not.toBeInTheDocument()
  },
}

export const DockerImage: Story = {
  decorators: [moduleMetadata({ providers: [withRoute('docker', 'web')] })],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByRole('heading', { name: 'web' })).toBeInTheDocument()
    expect(canvasElement.querySelector('app-copyable-command pre')).toHaveTextContent(
      'docker pull localhost:4200/u/alice/ui-kit/web:latest',
    )
    expect(canvas.getByText('56 téléchargements cette semaine')).toBeInTheDocument()
    expect(canvas.getByRole('columnheader', { name: 'Taille' })).toBeInTheDocument()
    expect(canvas.getByRole('cell', { name: '46,0 Mo' })).toBeInTheDocument()
    expect(canvas.getByRole('cell', { name: '—' })).toBeInTheDocument()
    expect(canvas.getByRole('cell', { name: 'latest' })).toBeInTheDocument()
    expect(canvas.getByRole('cell', { name: 'v1.0.0' })).toBeInTheDocument()
    expect(canvas.getByRole('columnheader', { name: 'Tag' })).toBeInTheDocument()
  },
}

export const DockerImageWithoutDownloads: Story = {
  decorators: [
    moduleMetadata({ providers: [withRoute('docker', 'web')] }),
    withRepositories({ dockerImageDetails: () => of({ ...DOCKER_DETAILS, downloads_7d: 0 }) }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByRole('heading', { name: 'web' })).toBeInTheDocument()
    expect(canvas.queryByText(/téléchargement/)).toBeNull()
  },
}

export const DockerImageWithoutTags: Story = {
  decorators: [
    moduleMetadata({ providers: [withRoute('docker', 'web')] }),
    withRepositories({ dockerImageDetails: () => of({ ...DOCKER_DETAILS, tags: [] }) }),
  ],
  play: async ({ canvasElement }) => {
    await within(canvasElement).findByRole('heading', { name: 'web' })
    expect(canvasElement.querySelector('app-copyable-command pre')).toHaveTextContent(
      /^docker pull localhost:4200\/u\/alice\/ui-kit\/web$/,
    )
  },
}

export const Loading: Story = {
  decorators: [withRepositories({ getByOwner: () => NEVER })],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByRole('status')).toHaveTextContent('Chargement…')
    expect(canvas.queryByRole('alert')).not.toBeInTheDocument()
  },
}

export const NotFound: Story = {
  decorators: [
    withRepositories({
      npmPackageDetails: () => throwError(() => new HttpErrorResponse({ status: 404 })),
    }),
  ],
  play: async ({ canvasElement }) => {
    await waitFor(() =>
      expect(within(canvasElement).getByRole('alert')).toHaveTextContent('Package introuvable.'),
    )
  },
}

export const RepositoryNotFound: Story = {
  decorators: [
    withRepositories({
      getByOwner: () => throwError(() => new HttpErrorResponse({ status: 404 })),
    }),
  ],
  play: async ({ canvasElement }) => {
    await waitFor(() =>
      expect(within(canvasElement).getByRole('alert')).toHaveTextContent('Package introuvable.'),
    )
  },
}

export const LoadFailed: Story = {
  decorators: [
    withRepositories({
      npmPackageDetails: () => throwError(() => new HttpErrorResponse({ status: 500 })),
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() =>
      expect(canvas.getByRole('alert')).toHaveTextContent('Échec du chargement du package.'),
    )
    expect(canvas.queryByText('Package introuvable.')).not.toBeInTheDocument()
  },
}

export const DockerLoadFailed: Story = {
  decorators: [
    moduleMetadata({ providers: [withRoute('docker', 'web')] }),
    withRepositories({
      dockerImageDetails: () => throwError(() => new Error('network error')),
    }),
  ],
  play: async ({ canvasElement }) => {
    await waitFor(() =>
      expect(within(canvasElement).getByRole('alert')).toHaveTextContent(
        'Échec du chargement du package.',
      ),
    )
  },
}

export const OrganizationNpmPackage: Story = {
  decorators: [moduleMetadata({ providers: [withOrgRoute('npm', '@acme/button')] })],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByRole('heading', { name: '@acme/button' })).toBeInTheDocument()
    expect(canvas.getByRole('cell', { name: '2.1.0' })).toBeInTheDocument()
    expect(canvas.getByRole('link', { name: '← Retour au dépôt' })).toHaveAttribute(
      'href',
      expect.stringMatching(/\/o\/acme\/ui-kit$/),
    )
  },
}

export const OrganizationRepositoryNotFound: Story = {
  decorators: [
    moduleMetadata({ providers: [withOrgRoute('npm', '@acme/button')] }),
    withRepositories({
      getByOrg: () => throwError(() => new HttpErrorResponse({ status: 404 })),
    }),
  ],
  play: async ({ canvasElement }) => {
    await waitFor(() =>
      expect(within(canvasElement).getByRole('alert')).toHaveTextContent('Package introuvable.'),
    )
  },
}
