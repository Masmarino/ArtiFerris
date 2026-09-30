import { applicationConfig, type Meta, type StoryObj } from '@storybook/angular-vite'
import { provideRouter } from '@angular/router'
import { expect, within } from 'storybook/test'
import { UsageInstructions } from './usage-instructions'
import type { RepositorySummary } from '../domain/repository.entity'

const REPO: RepositorySummary = {
  id: 'repo-1',
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
}

const meta: Meta<UsageInstructions> = {
  title: 'Repositories/UsageInstructions',
  component: UsageInstructions,
  decorators: [applicationConfig({ providers: [provideRouter([])] })],
  args: { repository: REPO },
}
export default meta

type Story = StoryObj<UsageInstructions>

export const NpmHosted: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(canvas.getByText(/registry=.*\/npm\/acme-npm\//)).toBeInTheDocument()
    expect(canvas.getByText('npm publish')).toBeInTheDocument()
    expect(canvas.getByText('npm install')).toBeInTheDocument()
    expect(canvas.queryByText(/docker/)).not.toBeInTheDocument()
  },
}

export const NpmProxy: Story = {
  args: { repository: { ...REPO, repo_type: 'proxy', remote_url: 'https://registry.npmjs.org' } },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(canvas.getByText('npm install')).toBeInTheDocument()
    expect(canvas.queryByText('npm publish')).not.toBeInTheDocument()
  },
}

export const DockerHosted: Story = {
  args: { repository: { ...REPO, name: 'acme-docker', format: 'docker' } },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(canvas.getByText(/docker login/)).toBeInTheDocument()
    expect(canvas.getByText(/docker push .*\/acme-docker\/mon-image:latest/)).toBeInTheDocument()
    expect(canvas.getByText(/docker pull .*\/acme-docker\//)).toBeInTheDocument()
    expect(canvas.queryByText(/npm/)).not.toBeInTheDocument()
  },
}

export const DockerProxy: Story = {
  args: {
    repository: {
      ...REPO,
      name: 'dockerhub-mirror',
      format: 'docker',
      repo_type: 'proxy',
      remote_url: 'https://registry-1.docker.io',
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(canvas.getByText(/docker pull .*\/dockerhub-mirror\//)).toBeInTheDocument()
    expect(canvas.queryByText(/docker push/)).not.toBeInTheDocument()
  },
}

export const PublicViewLoginLink: Story = {
  args: { apiTokenLink: '/login', apiTokenLinkLabel: 'Se connecter' },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(canvas.getByRole('link', { name: 'Se connecter' })).toHaveAttribute(
      'href',
      expect.stringMatching(/\/login$/),
    )
    expect(canvas.queryByRole('link', { name: 'Créer un token API' })).not.toBeInTheDocument()
  },
}

export const DefaultTokenLink: Story = {
  play: async ({ canvasElement }) => {
    expect(within(canvasElement).getByRole('link', { name: 'Créer un token API' })).toHaveAttribute(
      'href',
      expect.stringMatching(/\/account$/),
    )
  },
}
