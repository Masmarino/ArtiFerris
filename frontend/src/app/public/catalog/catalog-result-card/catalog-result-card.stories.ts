import { applicationConfig, type Meta, type StoryObj } from '@storybook/angular-vite'
import { provideRouter } from '@angular/router'
import { expect, spyOn, userEvent, waitFor, within } from 'storybook/test'
import { catalogEntry, dockerEntry } from '../testing/catalog.fixtures'
import { CatalogResultCard } from './catalog-result-card'

const meta: Meta<CatalogResultCard> = {
  title: 'Public/Catalog/CatalogResultCard',
  component: CatalogResultCard,
  decorators: [applicationConfig({ providers: [provideRouter([])] })],
  args: { entry: catalogEntry() },
}
export default meta

type Story = StoryObj<CatalogResultCard>

export const NpmPackage: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(canvas.getByRole('link', { name: 'hangar-demo' })).toHaveAttribute(
      'href',
      expect.stringMatching(/\/@admin\/test-npm\/packages\/npm\/hangar-demo$/),
    )
    expect(canvas.getByRole('link', { name: 'admin' })).toHaveAttribute(
      'href',
      expect.stringMatching(/\/@admin$/),
    )
    expect(canvas.getByRole('link', { name: 'test-npm' })).toHaveAttribute(
      'href',
      expect.stringMatching(/\/@admin\/test-npm$/),
    )
    expect(canvas.getByText('1.1.0')).toBeInTheDocument()
    expect(canvas.getByText('demo')).toBeInTheDocument()
    expect(canvasElement.querySelector('pre')).toHaveTextContent(
      'npm install hangar-demo --registry http://localhost:4200/npm/u/admin/test-npm/',
    )
  },
}

export const DockerImage: Story = {
  args: { entry: dockerEntry() },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(canvas.getByRole('link', { name: 'team/api' })).toHaveAttribute(
      'href',
      expect.stringMatching(/\/o\/acme\/images\/packages\/docker\/team%2Fapi$/),
    )
    expect(canvas.getByRole('link', { name: 'Acme Corp' })).toHaveAttribute(
      'href',
      expect.stringMatching(/\/o\/acme$/),
    )
    expect(canvas.getByRole('link', { name: 'images' })).toHaveAttribute(
      'href',
      expect.stringMatching(/\/o\/acme\/images$/),
    )
    expect(canvasElement.querySelector('pre')).toHaveTextContent(
      'docker pull localhost:4200/o/acme/images/team/api:v2.0.1',
    )
  },
}

export const WithDownloads: Story = {
  args: { entry: catalogEntry({ downloads_7d: 42 }) },
  play: async ({ canvasElement }) => {
    expect(within(canvasElement).getByText('42 téléchargements cette semaine')).toBeInTheDocument()
  },
}

export const SingleDownload: Story = {
  args: { entry: dockerEntry({ downloads_7d: 1 }) },
  play: async ({ canvasElement }) => {
    expect(within(canvasElement).getByText('1 téléchargement cette semaine')).toBeInTheDocument()
  },
}

export const LargeDownloads: Story = {
  args: { entry: catalogEntry({ downloads_7d: 1_250_000 }) },
  play: async ({ canvasElement }) => {
    expect(
      within(canvasElement).getByText(/^1\s250\s000 téléchargements cette semaine$/),
    ).toBeInTheDocument()
  },
}

export const NoDownloads: Story = {
  play: async ({ canvasElement }) => {
    expect(within(canvasElement).queryByText(/téléchargement/)).toBeNull()
  },
}

export const DockerImageWithoutTag: Story = {
  args: { entry: dockerEntry({ latest: null }) },
  play: async ({ canvasElement }) => {
    expect(canvasElement.querySelector('pre')).toHaveTextContent(
      /^docker pull localhost:4200\/o\/acme\/images\/team\/api$/,
    )
  },
}

export const Minimal: Story = {
  args: { entry: catalogEntry({ description: null, keywords: [], latest: null }) },
  play: async ({ canvasElement }) => {
    expect(canvasElement.querySelector('.result-card__description')).toBeNull()
    expect(canvasElement.querySelector('.result-card__keywords')).toBeNull()
    expect(canvasElement.querySelector('gbt-badge')).toBeNull()
  },
}

export const LongContent: Story = {
  args: {
    entry: catalogEntry({
      name: '@a-very-long-scope/an-equally-long-package-name-that-keeps-going',
      description:
        'A description that goes on for far longer than two lines so the clamp has something to cut. '.repeat(
          4,
        ),
      keywords: ['one', 'two', 'three', 'four', 'five', 'six', 'seven', 'eight', 'nine', 'ten'],
    }),
  },
}

export const CopyInstallCommand: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const writeText = spyOn(navigator.clipboard, 'writeText').mockResolvedValue(undefined)

    await userEvent.click(
      canvas.getByRole('button', { name: "Copier la commande d'installation de hangar-demo" }),
    )

    await waitFor(() =>
      expect(writeText).toHaveBeenCalledWith(
        'npm install hangar-demo --registry http://localhost:4200/npm/u/admin/test-npm/',
      ),
    )
    expect(await canvas.findByText('Commande copiée')).toBeInTheDocument()
    writeText.mockRestore()
  },
}

export const CopyFailed: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const writeText = spyOn(navigator.clipboard, 'writeText').mockRejectedValue(new Error('denied'))

    await userEvent.click(
      canvas.getByRole('button', { name: "Copier la commande d'installation de hangar-demo" }),
    )

    expect(await canvas.findByText(/Copie impossible/)).toBeInTheDocument()
    writeText.mockRestore()
  },
}
