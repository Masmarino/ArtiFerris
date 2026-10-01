import { HttpErrorResponse } from '@angular/common/http'
import { moduleMetadata, type Meta, type StoryObj } from '@storybook/angular-vite'
import { expect, fn, userEvent, waitFor, within } from 'storybook/test'
import { NEVER, of, throwError } from 'rxjs'
import { CreateOrganizationModal } from './create-organization-modal'
import { OrganizationsService } from '../application/organizations.service'
import { ToastService } from '../../shared/toast.service'
import type { OrganizationSummary } from '../domain/organization.entity'

const CREATED_ORG: OrganizationSummary = {
  id: 'org-acme',
  slug: 'acme',
  display_name: 'Acme Corp',
  is_public: false,
}

function fakeOrgs(overrides: Partial<OrganizationsService> = {}): Partial<OrganizationsService> {
  return { create: fn(() => of(CREATED_ORG)), ...overrides }
}

const toast = { success: fn(), error: fn() }

function withOrgs(orgs: Partial<OrganizationsService>) {
  return moduleMetadata({ providers: [{ provide: OrganizationsService, useValue: orgs }] })
}

async function fillForm(canvas: ReturnType<typeof within>) {
  // GbtInput appends " *" to required-field labels: match by prefix.
  await userEvent.type(await canvas.findByLabelText(/^Sous-domaine/), 'acme')
  await userEvent.type(canvas.getByLabelText(/^Nom/), 'Acme Corp')
}

const meta: Meta<CreateOrganizationModal> = {
  title: 'Admin/CreateOrganizationModal',
  component: CreateOrganizationModal,
  args: { created: fn(), cancelled: fn() },
  beforeEach: () => {
    toast.success.mockClear()
    toast.error.mockClear()
  },
  decorators: [
    moduleMetadata({
      providers: [
        { provide: OrganizationsService, useValue: fakeOrgs() },
        { provide: ToastService, useValue: toast },
      ],
    }),
  ],
}
export default meta

type Story = StoryObj<CreateOrganizationModal>

export const Empty: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(
      await canvas.findByRole('heading', { name: 'Nouvelle organisation' }),
    ).toBeInTheDocument()
    expect(canvas.getByRole('button', { name: 'Créer' })).toBeDisabled()

    await userEvent.type(canvas.getByLabelText(/^Sous-domaine/), 'acme')
    expect(canvas.getByRole('button', { name: 'Créer' })).toBeDisabled()

    await userEvent.type(canvas.getByLabelText(/^Nom/), 'Acme Corp')
    await waitFor(() => expect(canvas.getByRole('button', { name: 'Créer' })).toBeEnabled())
  },
}

const createSuccess = fakeOrgs()
export const CreatingAnOrganization: Story = {
  decorators: [withOrgs(createSuccess)],
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await fillForm(canvas)
    await userEvent.click(canvas.getByRole('button', { name: 'Créer' }))

    await waitFor(() => expect(createSuccess.create).toHaveBeenCalledWith('acme', 'Acme Corp'))
    expect(args.created).toHaveBeenCalledTimes(1)
    expect(toast.success).toHaveBeenCalledWith('Organisation « Acme Corp » créée.')
  },
}

const createInFlight = fakeOrgs({ create: fn(() => NEVER) })
export const Creating: Story = {
  decorators: [withOrgs(createInFlight)],
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await fillForm(canvas)
    await userEvent.click(canvas.getByRole('button', { name: 'Créer' }))

    const button = await canvas.findByRole('button', { name: /Créer/ })
    await waitFor(() => expect(button).toHaveAttribute('aria-busy', 'true'))
    expect(button).toBeDisabled()
    expect(createInFlight.create).toHaveBeenCalledTimes(1)
    expect(args.created).not.toHaveBeenCalled()
  },
}

const createRejected = fakeOrgs({
  create: fn(() =>
    throwError(
      () =>
        new HttpErrorResponse({ status: 400, error: { error: 'ce sous-domaine est déjà pris' } }),
    ),
  ),
})
export const CreationRejectedByTheServer: Story = {
  decorators: [withOrgs(createRejected)],
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await fillForm(canvas)
    await userEvent.click(canvas.getByRole('button', { name: 'Créer' }))

    await waitFor(() => expect(toast.error).toHaveBeenCalledWith('ce sous-domaine est déjà pris'))
    expect(args.created).not.toHaveBeenCalled()
    expect(toast.success).not.toHaveBeenCalled()
    // The form stays filled and usable for a retry.
    expect(canvas.getByLabelText(/^Sous-domaine/)).toHaveValue('acme')
    expect(canvas.getByRole('button', { name: 'Créer' })).toBeEnabled()
  },
}

const createBroken = fakeOrgs({ create: fn(() => throwError(() => new Error('network'))) })
export const CreationFailedWithoutAReason: Story = {
  decorators: [withOrgs(createBroken)],
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await fillForm(canvas)
    await userEvent.click(canvas.getByRole('button', { name: 'Créer' }))

    await waitFor(() =>
      expect(toast.error).toHaveBeenCalledWith("Échec de la création de l'organisation."),
    )
    expect(args.created).not.toHaveBeenCalled()
  },
}

export const Cancelling: Story = {
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('button', { name: 'Fermer' }))

    await waitFor(() => expect(args.cancelled).toHaveBeenCalledTimes(1))
    expect(args.created).not.toHaveBeenCalled()
  },
}
