import { HttpErrorResponse } from '@angular/common/http'
import { signal } from '@angular/core'
import { moduleMetadata, type Meta, type StoryObj } from '@storybook/angular-vite'
import { expect, fn, userEvent, waitFor, within } from 'storybook/test'
import { NEVER, of, throwError } from 'rxjs'
import { CreateRepositoryModal } from './create-repository-modal'
import { RepositoriesService } from '../application/repositories.service'
import { ToastService } from '../../shared/toast.service'
import { MeService } from '../../shell/application/me.service'
import type { RepositorySummary } from '../domain/repository.entity'

const CREATED: RepositorySummary = {
  id: 'repo-new',
  name: 'my-repo',
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
const NPM_A: RepositorySummary = { ...CREATED, id: 'npm-a', name: 'npm-a' }
const NPM_B: RepositorySummary = { ...CREATED, id: 'npm-b', name: 'npm-b' }
const DOCKER_A: RepositorySummary = {
  ...CREATED,
  id: 'docker-a',
  name: 'docker-a',
  format: 'docker',
}

type Canvas = ReturnType<typeof within>

/** Fresh spies per story so call counts never leak from one story into another. */
function scenario(
  options: { repositories?: Partial<RepositoriesService>; isSuperAdmin?: boolean } = {},
) {
  const repositories = {
    list: () => of([NPM_A, NPM_B, DOCKER_A]),
    create: fn(() => of(CREATED)),
    setVisibility: fn(() => of(undefined)),
    ...options.repositories,
  } as Partial<RepositoriesService> & {
    create: ReturnType<typeof fn>
    setVisibility: ReturnType<typeof fn>
  }
  const toast = { success: fn(), error: fn() }
  const decorator = moduleMetadata({
    providers: [
      { provide: RepositoriesService, useValue: repositories },
      { provide: ToastService, useValue: toast },
      { provide: MeService, useValue: { isSuperAdmin: signal(options.isSuperAdmin ?? false) } },
    ],
  })
  return { repositories, toast, decorator }
}

const QUOTA_LABEL = 'Quota (Mo, vide = illimité)'
const RETENTION_LABEL = 'Conserver les N dernières versions/étiquettes (vide = désactivé)'

async function fillName(canvas: Canvas, name: string) {
  await userEvent.type(await canvas.findByLabelText(/^Nom( \*)?$/), name)
}
async function pickOption(canvas: Canvas, select: string, option: string) {
  await userEvent.click(await canvas.findByRole('combobox', { name: select }))
  await userEvent.click(await canvas.findByRole('option', { name: option }))
}
async function addMember(canvas: Canvas, name: string) {
  await pickOption(canvas, 'Ajouter un dépôt membre', name)
  await userEvent.click(canvas.getByRole('button', { name: 'Ajouter' }))
}
function memberNames(canvas: Canvas): string[] {
  return canvas
    .queryAllByRole('listitem')
    .map((item: HTMLElement) => item.querySelector('span')!.textContent!)
}

const meta: Meta<CreateRepositoryModal> = {
  title: 'Repositories/CreateRepositoryModal',
  component: CreateRepositoryModal,
  args: { created: fn(), cancelled: fn() },
  decorators: [scenario().decorator],
}
export default meta

type Story = StoryObj<CreateRepositoryModal>

/** A fresh form: hosted npm with no name yet, so it can't be submitted. */
export const Default: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByRole('heading', { name: 'Nouveau dépôt' })).toBeInTheDocument()
    expect(canvas.getByRole('button', { name: 'Créer' })).toBeDisabled()
    expect(canvas.queryByLabelText('Rendre public')).not.toBeInTheDocument()
    expect(canvas.queryByLabelText(/URL distante/)).not.toBeInTheDocument()
  },
}

export const NameIsRequired: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const create = await canvas.findByRole('button', { name: 'Créer' })
    expect(create).toBeDisabled()
    await fillName(canvas, 'my-repo')
    await waitFor(() => expect(create).toBeEnabled())
  },
}

export const InvalidQuota: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await fillName(canvas, 'my-repo')
    await userEvent.type(canvas.getByLabelText(QUOTA_LABEL), 'beaucoup')
    expect(
      await canvas.findByText('Doit être un nombre positif (ou vide pour illimité).'),
    ).toBeInTheDocument()
    expect(canvas.getByRole('button', { name: 'Créer' })).toBeDisabled()
  },
}

export const InvalidRetention: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await fillName(canvas, 'my-repo')
    await userEvent.type(canvas.getByLabelText(RETENTION_LABEL), '0')
    expect(
      await canvas.findByText('Doit être un entier positif (ou vide pour désactiver).'),
    ).toBeInTheDocument()
    expect(canvas.getByRole('button', { name: 'Créer' })).toBeDisabled()
  },
}

const hosted = scenario()
/** The quota is typed in MB and sent in bytes. */
export const CreatingAHostedRepository: Story = {
  decorators: [hosted.decorator],
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await fillName(canvas, 'my-repo')
    await userEvent.type(canvas.getByLabelText(QUOTA_LABEL), '10')
    await userEvent.type(canvas.getByLabelText(RETENTION_LABEL), '5')
    await userEvent.click(canvas.getByRole('button', { name: 'Créer' }))
    await waitFor(() => expect(args.created).toHaveBeenCalledOnce())
    expect(hosted.repositories.create).toHaveBeenCalledWith('my-repo', 'npm', 'hosted', null, {
      remoteUsername: null,
      remotePassword: null,
      groupMembers: [],
      quotaBytes: 10 * 1024 * 1024,
      retentionKeepLastN: 5,
    })
    expect(hosted.repositories.setVisibility).not.toHaveBeenCalled()
    expect(hosted.toast.success).toHaveBeenCalledWith('Dépôt « my-repo » créé.')
  },
}

const dockerRepo = scenario()
export const CreatingADockerRepository: Story = {
  decorators: [dockerRepo.decorator],
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await fillName(canvas, 'my-images')
    await pickOption(canvas, 'Format', 'docker')
    await userEvent.click(canvas.getByRole('button', { name: 'Créer' }))
    await waitFor(() => expect(args.created).toHaveBeenCalledOnce())
    expect(dockerRepo.repositories.create).toHaveBeenCalledWith(
      'my-images',
      'docker',
      'hosted',
      null,
      expect.objectContaining({ quotaBytes: null, retentionKeepLastN: null }),
    )
  },
}

const proxy = scenario()
/** A proxy asks for its upstream URL and optional credentials, and sends them along. */
export const CreatingAProxyRepository: Story = {
  decorators: [proxy.decorator],
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await fillName(canvas, 'npmjs-mirror')
    await pickOption(canvas, 'Type', 'proxy')
    await userEvent.type(
      await canvas.findByLabelText(/^URL distante/),
      'https://registry.npmjs.org',
    )
    await userEvent.type(
      canvas.getByLabelText("Nom d'utilisateur distant (optionnel)"),
      'svc-account',
    )
    await userEvent.type(
      canvas.getByLabelText('Mot de passe / token distant (optionnel)'),
      's3cret',
    )
    await userEvent.click(canvas.getByRole('button', { name: 'Créer' }))
    await waitFor(() => expect(args.created).toHaveBeenCalledOnce())
    expect(proxy.repositories.create).toHaveBeenCalledWith(
      'npmjs-mirror',
      'npm',
      'proxy',
      'https://registry.npmjs.org',
      expect.objectContaining({ remoteUsername: 'svc-account', remotePassword: 's3cret' }),
    )
  },
}

export const GroupWithoutCandidateMembers: Story = {
  decorators: [scenario({ repositories: { list: () => of([DOCKER_A]) } }).decorator],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await pickOption(canvas, 'Type', 'group')
    expect(
      await canvas.findByText("Aucun dépôt npm disponible à ajouter pour l'instant."),
    ).toBeInTheDocument()
    expect(canvas.queryByRole('combobox', { name: 'Ajouter un dépôt membre' })).toBeNull()
  },
}

const group = scenario()
/**
 * Members are picked from same-format repositories and can be reordered or removed before creating.
 */
export const CreatingAGroupRepository: Story = {
  decorators: [group.decorator],
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await fillName(canvas, 'all-npm')
    await pickOption(canvas, 'Type', 'group')

    expect(canvas.getByRole('button', { name: 'Ajouter' })).toBeDisabled()
    await userEvent.click(await canvas.findByRole('combobox', { name: 'Ajouter un dépôt membre' }))
    expect(await canvas.findByRole('option', { name: 'npm-a' })).toBeInTheDocument()
    expect(canvas.queryByRole('option', { name: 'docker-a' })).not.toBeInTheDocument()
    await userEvent.keyboard('{Escape}')

    await addMember(canvas, 'npm-b')
    await addMember(canvas, 'npm-a')
    expect(memberNames(canvas)).toEqual(['npm-b', 'npm-a'])

    const [first, second] = canvas.getAllByRole('listitem')
    expect(within(first).getByRole('button', { name: 'Monter' })).toBeDisabled()
    expect(within(second).getByRole('button', { name: 'Descendre' })).toBeDisabled()
    await userEvent.click(within(second).getByRole('button', { name: 'Monter' }))
    expect(memberNames(canvas)).toEqual(['npm-a', 'npm-b'])

    await userEvent.click(canvas.getByRole('button', { name: 'Créer' }))
    await waitFor(() => expect(args.created).toHaveBeenCalledOnce())
    expect(group.repositories.create).toHaveBeenCalledWith(
      'all-npm',
      'npm',
      'group',
      null,
      expect.objectContaining({ groupMembers: ['npm-a', 'npm-b'] }),
    )
  },
}

export const RemovingAGroupMember: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await pickOption(canvas, 'Type', 'group')
    await addMember(canvas, 'npm-a')
    await addMember(canvas, 'npm-b')
    await userEvent.click(
      within(canvas.getAllByRole('listitem')[0]).getByRole('button', { name: 'Retirer' }),
    )
    expect(memberNames(canvas)).toEqual(['npm-b'])
    // A removed repository becomes available to pick again.
    await userEvent.click(await canvas.findByRole('combobox', { name: 'Ajouter un dépôt membre' }))
    expect(await canvas.findByRole('option', { name: 'npm-a' })).toBeInTheDocument()
  },
}

/** A group only aggregates same-format repositories, so switching format clears the picks. */
export const ChangingFormatClearsGroupMembers: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await pickOption(canvas, 'Type', 'group')
    await addMember(canvas, 'npm-a')
    expect(memberNames(canvas)).toEqual(['npm-a'])
    await pickOption(canvas, 'Format', 'docker')
    await waitFor(() => expect(canvas.queryAllByRole('listitem')).toHaveLength(0))
    await userEvent.click(await canvas.findByRole('combobox', { name: 'Ajouter un dépôt membre' }))
    expect(await canvas.findByRole('option', { name: 'docker-a' })).toBeInTheDocument()
    expect(canvas.queryByRole('option', { name: 'npm-a' })).not.toBeInTheDocument()
  },
}

/** Only a super-admin can publish a repository, and only a hosted one. */
export const AsSuperAdmin: Story = {
  decorators: [scenario({ isSuperAdmin: true }).decorator],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByLabelText('Rendre public')).not.toBeChecked()
    await pickOption(canvas, 'Type', 'proxy')
    await waitFor(() => expect(canvas.queryByLabelText('Rendre public')).not.toBeInTheDocument())
    await pickOption(canvas, 'Type', 'group')
    expect(canvas.queryByLabelText('Rendre public')).not.toBeInTheDocument()
    await pickOption(canvas, 'Type', 'hosted')
    expect(await canvas.findByLabelText('Rendre public')).toBeInTheDocument()
  },
}

/** A ticked "Rendre public" doesn't survive a switch to a type that can't be public. */
export const PublicTickIsDroppedForProxy: Story = {
  decorators: [scenario({ isSuperAdmin: true }).decorator],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByLabelText('Rendre public'))
    expect(canvas.getByLabelText('Rendre public')).toBeChecked()
    await pickOption(canvas, 'Type', 'proxy')
    await pickOption(canvas, 'Type', 'hosted')
    expect(await canvas.findByLabelText('Rendre public')).not.toBeChecked()
  },
}

const publicRepo = scenario({ isSuperAdmin: true })
/**
 * Creation has no visibility field, so going public is a second call after the repository exists.
 */
export const CreatingAPublicRepository: Story = {
  decorators: [publicRepo.decorator],
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await fillName(canvas, 'my-repo')
    await userEvent.click(canvas.getByLabelText('Rendre public'))
    await userEvent.click(canvas.getByRole('button', { name: 'Créer' }))
    await waitFor(() => expect(args.created).toHaveBeenCalledOnce())
    expect(publicRepo.repositories.setVisibility).toHaveBeenCalledWith('repo-new', true)
    expect(publicRepo.toast.success).toHaveBeenCalledWith('Dépôt « my-repo » créé.')
    expect(publicRepo.toast.error).not.toHaveBeenCalled()
  },
}

const visibilityFails = scenario({
  isSuperAdmin: true,
  repositories: { setVisibility: fn(() => throwError(() => ({ error: {} }))) },
})
/** The repository exists even if the follow-up fails, so the modal still reports it as created. */
export const MakingItPublicFailed: Story = {
  decorators: [visibilityFails.decorator],
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await fillName(canvas, 'my-repo')
    await userEvent.click(canvas.getByLabelText('Rendre public'))
    await userEvent.click(canvas.getByRole('button', { name: 'Créer' }))
    await waitFor(() => expect(args.created).toHaveBeenCalledOnce())
    expect(visibilityFails.toast.error).toHaveBeenCalledWith(
      'Dépôt créé, mais échec du passage en public.',
    )
    expect(visibilityFails.toast.success).not.toHaveBeenCalled()
  },
}

const failing = scenario({
  repositories: {
    create: fn(() =>
      throwError(
        () =>
          new HttpErrorResponse({
            status: 400,
            error: { error: 'le dépôt « my-repo » existe déjà' },
          }),
      ),
    ),
  },
})
/** The server's own message is shown, the modal stays open and the form can be resubmitted. */
export const CreationFailed: Story = {
  decorators: [failing.decorator],
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await fillName(canvas, 'my-repo')
    await userEvent.click(canvas.getByRole('button', { name: 'Créer' }))
    await waitFor(() =>
      expect(failing.toast.error).toHaveBeenCalledWith('le dépôt « my-repo » existe déjà'),
    )
    expect(args.created).not.toHaveBeenCalled()
    expect(canvas.getByRole('button', { name: 'Créer' })).toBeEnabled()
  },
}

const failingWithoutMessage = scenario({
  repositories: { create: fn(() => throwError(() => new Error('network error'))) },
})
export const CreationFailedWithoutServerMessage: Story = {
  decorators: [failingWithoutMessage.decorator],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await fillName(canvas, 'my-repo')
    await userEvent.click(canvas.getByRole('button', { name: 'Créer' }))
    await waitFor(() =>
      expect(failingWithoutMessage.toast.error).toHaveBeenCalledWith(
        'Échec de la création du dépôt.',
      ),
    )
  },
}

const inFlight = scenario({ repositories: { create: fn(() => NEVER) } })
/** While the request is pending the button is busy and a second click can't submit again. */
export const Creating: Story = {
  decorators: [inFlight.decorator],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await fillName(canvas, 'my-repo')
    await userEvent.click(canvas.getByRole('button', { name: 'Créer' }))
    const busy = await canvas.findByRole('button', { name: /Créer/ })
    await waitFor(() => expect(busy).toBeDisabled())
    expect(busy).toHaveAttribute('aria-busy', 'true')
    expect(inFlight.repositories.create).toHaveBeenCalledOnce()
  },
}

export const Cancelling: Story = {
  play: async ({ canvasElement, args }) => {
    await userEvent.click(await within(canvasElement).findByRole('button', { name: 'Fermer' }))
    expect(args.cancelled).toHaveBeenCalledOnce()
    expect(args.created).not.toHaveBeenCalled()
  },
}
