import { HttpErrorResponse } from '@angular/common/http'
import { moduleMetadata, type Meta, type StoryObj } from '@storybook/angular-vite'
import { userEvent, within } from 'storybook/test'
import { of, throwError } from 'rxjs'
import { CreateUserProjectModal } from './create-user-project-modal'
import { PersonalRepositoryService } from '../application/personal-repository.service'
import { RepositoriesService } from '../application/repositories.service'
import type { RepositorySummary } from '../domain/repository.entity'

const CREATED_PROJECT: RepositorySummary = {
  id: 'repo-1',
  name: 'my-project',
  format: 'npm',
  repo_type: 'hosted',
  remote_url: null,
  remote_credentials_set: false,
  group_members: [],
  quota_bytes: null,
  retention_keep_last_n: null,
  is_public: false,
  my_role: 'admin',
  organization_id: 'org-1',
  owner_name: 'florian',
  owner_is_personal: true,
}

function fakePersonalRepositoryService(
  overrides: Partial<PersonalRepositoryService> = {},
): Partial<PersonalRepositoryService> {
  return { createProject: () => of(CREATED_PROJECT), ...overrides }
}
function fakeRepositoriesService(
  overrides: Partial<RepositoriesService> = {},
): Partial<RepositoriesService> {
  return { setVisibility: () => of(undefined), ...overrides }
}

async function fillProjectName(canvasElement: HTMLElement, name: string) {
  const canvas = within(canvasElement)
  // GbtInput appends " *" to required-field labels: match by prefix.
  await userEvent.type(await canvas.findByLabelText(/^Nom/), name)
}

const meta: Meta<CreateUserProjectModal> = {
  title: 'Repositories/CreateUserProjectModal',
  component: CreateUserProjectModal,
  decorators: [
    moduleMetadata({
      providers: [
        { provide: PersonalRepositoryService, useValue: fakePersonalRepositoryService() },
        { provide: RepositoriesService, useValue: fakeRepositoriesService() },
      ],
    }),
  ],
}
export default meta

type Story = StoryObj<CreateUserProjectModal>

export const Default: Story = {}

export const CreatingAPublicProject: Story = {
  play: async ({ canvasElement }) => {
    await fillProjectName(canvasElement, 'my-project')
    const canvas = within(canvasElement)
    await userEvent.click(canvas.getByLabelText('Rendre public'))
    await userEvent.click(canvas.getByRole('button', { name: 'Créer' }))
  },
}

export const CreationFailed: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: PersonalRepositoryService,
          useValue: fakePersonalRepositoryService({
            createProject: () =>
              throwError(
                () =>
                  new HttpErrorResponse({
                    status: 409,
                    error: { error: 'un projet nommé « my-project » existe déjà' },
                  }),
              ),
          }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    await fillProjectName(canvasElement, 'my-project')
    // The error toast is rendered by the app shell, absent here: nothing more to assert.
    await userEvent.click(within(canvasElement).getByRole('button', { name: 'Créer' }))
  },
}
