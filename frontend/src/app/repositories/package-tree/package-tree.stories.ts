import {
  applicationConfig,
  moduleMetadata,
  type Meta,
  type StoryObj,
} from '@storybook/angular-vite'
import { provideRouter } from '@angular/router'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { NEVER, of, throwError } from 'rxjs'
import { PackageTree } from './package-tree'
import { RepositoriesService } from '../application/repositories.service'
import type { RepositoryPackages } from '../domain/repository.entity'

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
        {
          version: '1.1.0',
          published_at: '2026-08-01T00:00:00Z',
          size_bytes: 40_000,
          deprecated: true,
        },
      ],
      truncated: false,
      vulnerability_summary: { critical: 0, high: 2, medium: 0, low: 0 },
    },
    {
      name: 'left-pad',
      versions: [
        {
          version: '1.3.0',
          published_at: '2026-07-01T00:00:00Z',
          size_bytes: 3_000,
          deprecated: false,
        },
      ],
      truncated: false,
      vulnerability_summary: { critical: 0, high: 0, medium: 0, low: 0 },
    },
  ],
}

const NPM_PACKAGES_PAGE_2 = {
  name: 'zod',
  versions: [
    {
      version: '3.0.0',
      published_at: '2026-06-01T00:00:00Z',
      size_bytes: 9_000,
      deprecated: false,
    },
  ],
  truncated: false,
  vulnerability_summary: { critical: 0, high: 0, medium: 0, low: 0 },
}

const DOCKER_IMAGES: RepositoryPackages = {
  format: 'docker',
  images: [
    {
      image_name: 'acme-api',
      tags: ['latest', 'v1.0.0', 'v1.1.0'],
      truncated: false,
      vulnerability_summary: { critical: 1, high: 0, medium: 0, low: 4 },
    },
  ],
}

function fakeRepositories(
  overrides: Partial<RepositoriesService> = {},
): Partial<RepositoriesService> {
  return { packages: () => of(NPM_PACKAGES), ...overrides }
}
function withPackages(tree: RepositoryPackages) {
  return moduleMetadata({
    providers: [
      { provide: RepositoriesService, useValue: fakeRepositories({ packages: () => of(tree) }) },
    ],
  })
}

const meta: Meta<PackageTree> = {
  title: 'Repositories/PackageTree',
  component: PackageTree,
  args: { repositoryId: 'repo-1' },
  decorators: [
    applicationConfig({ providers: [provideRouter([])] }),
    moduleMetadata({
      providers: [{ provide: RepositoriesService, useValue: fakeRepositories() }],
    }),
  ],
}
export default meta

type Story = StoryObj<PackageTree>

/** Each npm package with its version count, vulnerability badge and a link to its detail page. */
export const NpmPackages: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const link = await canvas.findByRole('link', { name: /@acme\/ui/ })
    expect(link).toHaveTextContent('2')
    expect(link).toHaveAttribute(
      'href',
      expect.stringContaining('/repositories/repo-1/packages/npm/'),
    )
    expect(within(link).getByLabelText('2 vulnérabilités de sévérité élevée')).toBeInTheDocument()
    expect(canvas.getByRole('link', { name: /left-pad/ })).toHaveTextContent('1')
  },
}

export const DockerImages: Story = {
  decorators: [withPackages(DOCKER_IMAGES)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const link = await canvas.findByRole('link', { name: /acme-api/ })
    expect(link).toHaveTextContent('3')
    expect(link).toHaveAttribute(
      'href',
      expect.stringMatching(/\/repositories\/repo-1\/packages\/docker\/acme-api$/),
    )
    expect(within(link).getByLabelText('1 vulnérabilité de sévérité critique')).toBeInTheDocument()
  },
}

const FULL_VERSIONS = Array.from({ length: 200 }, (_, i) => ({
  version: `1.0.${199 - i}`,
  published_at: '2026-09-01T00:00:00Z',
  size_bytes: 1_000,
  deprecated: false,
}))

/** A package whose versions were cut at the server's cap says so; the untouched one doesn't. */
export const TruncatedPackage: Story = {
  decorators: [
    withPackages({
      format: 'npm',
      packages: [
        {
          name: 'busy-lib',
          versions: FULL_VERSIONS,
          truncated: true,
          vulnerability_summary: { critical: 0, high: 0, medium: 0, low: 0 },
        },
        NPM_PACKAGES.packages[1],
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await canvas.findByRole('link', { name: /busy-lib/ })
    expect(canvas.getAllByText('Seules les 200 plus récentes sont listées')).toHaveLength(1)
  },
}

export const TruncatedDockerImage: Story = {
  decorators: [
    withPackages({
      format: 'docker',
      images: [
        {
          image_name: 'busy-image',
          tags: Array.from({ length: 100 }, (_, i) => `t${i}`),
          truncated: true,
          vulnerability_summary: { critical: 0, high: 0, medium: 0, low: 0 },
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    expect(
      await within(canvasElement).findByText('Seules les 100 plus récentes sont listées'),
    ).toBeInTheDocument()
  },
}

/** The public view links under `/@user/repo` instead of `/repositories/:id`. */
export const CustomBasePath: Story = {
  args: { basePath: ['/@alice', 'ui-kit'] },
  decorators: [withPackages(DOCKER_IMAGES)],
  play: async ({ canvasElement }) => {
    expect(await within(canvasElement).findByRole('link', { name: /acme-api/ })).toHaveAttribute(
      'href',
      expect.stringMatching(/\/@alice\/ui-kit\/packages\/docker\/acme-api$/),
    )
  },
}

/** A cursor means more packages exist: "Charger plus" fetches them and appends them. */
export const MorePackagesToLoad: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: RepositoriesService,
          useValue: fakeRepositories({
            packages: (_id: string, after?: string | null) =>
              of(
                after
                  ? {
                      format: 'npm' as const,
                      packages: [NPM_PACKAGES_PAGE_2],
                      next_after: null,
                    }
                  : { ...NPM_PACKAGES, next_after: 'left-pad' },
              ),
          }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByRole('link', { name: /left-pad/ })).toBeInTheDocument()
    expect(canvas.queryByRole('link', { name: /zod/ })).not.toBeInTheDocument()
    await userEvent.click(canvas.getByRole('button', { name: 'Charger plus' }))
    expect(await canvas.findByRole('link', { name: /zod/ })).toBeInTheDocument()
    expect(canvas.queryByRole('button', { name: 'Charger plus' })).not.toBeInTheDocument()
  },
}

export const EmptyNpmRepository: Story = {
  decorators: [withPackages({ format: 'npm', packages: [] })],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByText('Aucun package')).toBeInTheDocument()
    expect(canvas.queryByRole('link')).not.toBeInTheDocument()
  },
}

export const EmptyDockerRepository: Story = {
  decorators: [withPackages({ format: 'docker', images: [] })],
  play: async ({ canvasElement }) => {
    expect(await within(canvasElement).findByText('Aucune image')).toBeInTheDocument()
  },
}

export const Loading: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        { provide: RepositoriesService, useValue: fakeRepositories({ packages: () => NEVER }) },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByRole('status')).toHaveTextContent('Chargement…')
    expect(canvas.queryByRole('link')).not.toBeInTheDocument()
  },
}

export const LoadFailed: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: RepositoriesService,
          useValue: fakeRepositories({ packages: () => throwError(() => new Error('down')) }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    await waitFor(() =>
      expect(within(canvasElement).getByRole('alert')).toHaveTextContent(
        'Échec du chargement des packages.',
      ),
    )
  },
}
