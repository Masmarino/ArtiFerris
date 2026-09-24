import { applicationConfig, type Meta, type StoryObj } from '@storybook/angular-vite'
import { provideRouter } from '@angular/router'
import { expect, within } from 'storybook/test'
import { ShareRepositoryLink } from './share-repository-link'
import type { RepositorySummary } from '../domain/repository.entity'

const REPO: RepositorySummary = {
  id: 'repo-1',
  name: 'libs',
  format: 'npm',
  repo_type: 'hosted',
  remote_url: null,
  remote_credentials_set: false,
  group_members: [],
  quota_bytes: null,
  retention_keep_last_n: null,
  is_public: true,
  my_role: 'admin',
  organization_id: 'org-1',
  owner_name: 'alice',
  owner_is_personal: true,
  public_path: '/@alice/libs',
}

const meta: Meta<ShareRepositoryLink> = {
  title: 'Repositories/ShareRepositoryLink',
  component: ShareRepositoryLink,
  decorators: [applicationConfig({ providers: [provideRouter([])] })],
  args: { repository: REPO },
}
export default meta

type Story = StoryObj<ShareRepositoryLink>

/** A public personal repository: the full link, ready to copy, and a way to open the page itself. */
export const PersonalPublic: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(canvas.getByText(/\/@alice\/libs$/)).toBeInTheDocument()
    expect(canvas.getByRole('link', { name: /Voir la page publique/ })).toHaveAttribute(
      'href',
      expect.stringMatching(/\/@alice\/libs$/),
    )
  },
}

export const OrganizationPublic: Story = {
  args: {
    repository: {
      ...REPO,
      name: 'design-system',
      owner_is_personal: false,
      owner_name: 'Acme Corp',
      public_path: '/o/acme/design-system',
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(canvas.getByText(/\/o\/acme\/design-system$/)).toBeInTheDocument()
  },
}

/** A private repository has no link to share yet; a read-only viewer isn't told about a Paramètres tab they don't have. */
export const Private: Story = {
  args: { repository: { ...REPO, is_public: false, public_path: null } },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(canvas.getByText(/Ce dépôt est privé/)).toBeInTheDocument()
    expect(canvas.queryByRole('link')).not.toBeInTheDocument()
    expect(canvas.queryByText(/Paramètres/)).not.toBeInTheDocument()
  },
}

/** An admin sees where to go to make it public. */
export const PrivateAsAdmin: Story = {
  args: { repository: { ...REPO, is_public: false, public_path: null }, canManageVisibility: true },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(canvas.getByText(/l'onglet Paramètres/)).toBeInTheDocument()
  },
}
