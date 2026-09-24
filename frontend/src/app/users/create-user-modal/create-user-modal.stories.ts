import { HttpErrorResponse } from '@angular/common/http'
import { moduleMetadata, type Meta, type StoryObj } from '@storybook/angular-vite'
import { expect, fn, userEvent, waitFor, within } from 'storybook/test'
import { NEVER, of, throwError } from 'rxjs'
import { CreateUserModal } from './create-user-modal'
import { UsersService } from '../application/users.service'
import { ToastService } from '../../shared/toast.service'
import type { UserSummary } from '../domain/user.entity'

const CREATED: UserSummary = {
  id: 'u9',
  username: 'alice',
  is_super_admin: false,
  organization_id: 'org-acme',
  email: 'alice@example.com',
  invitation_pending: true,
}

function fakeUsers(overrides: Partial<UsersService> = {}): Partial<UsersService> {
  return { create: fn(() => of(CREATED)), ...overrides }
}

function withServices(users: Partial<UsersService>, toasts = new ToastService()) {
  return moduleMetadata({
    providers: [
      { provide: UsersService, useValue: users },
      { provide: ToastService, useValue: toasts },
    ],
  })
}

const meta: Meta<CreateUserModal> = {
  title: 'Users/CreateUserModal',
  component: CreateUserModal,
  decorators: [withServices(fakeUsers())],
  args: { created: fn(), cancelled: fn() },
}
export default meta

type Story = StoryObj<CreateUserModal>

/** The "Inviter" button is disabled until both required fields are valid. */
export const Default: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByRole('heading', { name: 'Nouvel utilisateur' })).toBeInTheDocument()
    expect(canvas.getByRole('button', { name: 'Inviter' })).toBeDisabled()
    expect(canvas.getByRole('checkbox', { name: 'Super-administrateur' })).not.toBeChecked()
  },
}

export const InvalidEmail: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.type(await canvas.findByLabelText(/Nom d'utilisateur/), 'alice')
    await userEvent.type(canvas.getByLabelText(/Adresse e-mail/), 'not-an-email')
    await userEvent.tab()
    expect(canvas.getByRole('button', { name: 'Inviter' })).toBeDisabled()

    await userEvent.clear(canvas.getByLabelText(/Adresse e-mail/))
    await userEvent.type(canvas.getByLabelText(/Adresse e-mail/), 'alice@example.com')
    await waitFor(() => expect(canvas.getByRole('button', { name: 'Inviter' })).toBeEnabled())
  },
}

const successUsers = fakeUsers()
const successToasts = new ToastService()

export const InvitingAUser: Story = {
  decorators: [withServices(successUsers, successToasts)],
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await userEvent.type(await canvas.findByLabelText(/Nom d'utilisateur/), 'alice')
    await userEvent.type(canvas.getByLabelText(/Adresse e-mail/), 'alice@example.com')
    await userEvent.click(canvas.getByRole('button', { name: 'Inviter' }))

    await waitFor(() => expect(args.created).toHaveBeenCalledTimes(1))
    expect(successUsers.create).toHaveBeenCalledWith('alice', 'alice@example.com', false)
    expect(successToasts.toasts()).toEqual([
      expect.objectContaining({ variant: 'success', message: 'Utilisateur « alice » créé.' }),
    ])
    expect(args.cancelled).not.toHaveBeenCalled()
  },
}

const adminUsers = fakeUsers()

export const InvitingASuperAdmin: Story = {
  decorators: [withServices(adminUsers)],
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await userEvent.type(await canvas.findByLabelText(/Nom d'utilisateur/), 'root')
    await userEvent.type(canvas.getByLabelText(/Adresse e-mail/), 'root@example.com')
    await userEvent.click(canvas.getByRole('checkbox', { name: 'Super-administrateur' }))
    await userEvent.click(canvas.getByRole('button', { name: 'Inviter' }))

    await waitFor(() => expect(args.created).toHaveBeenCalledTimes(1))
    expect(adminUsers.create).toHaveBeenCalledWith('root', 'root@example.com', true)
  },
}

/** The request is in flight: the button is busy and cannot be clicked again. */
export const Submitting: Story = {
  decorators: [withServices(fakeUsers({ create: () => NEVER }))],
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await userEvent.type(await canvas.findByLabelText(/Nom d'utilisateur/), 'alice')
    await userEvent.type(canvas.getByLabelText(/Adresse e-mail/), 'alice@example.com')
    await userEvent.click(canvas.getByRole('button', { name: 'Inviter' }))

    const submit = await canvas.findByRole('button', { name: /Inviter/ })
    await waitFor(() => expect(submit).toHaveAttribute('aria-busy', 'true'))
    expect(submit).toBeDisabled()
    expect(args.created).not.toHaveBeenCalled()
  },
}

const conflictToasts = new ToastService()

/** The backend's own message (here a duplicate username) is surfaced and the modal stays open. */
export const ServerRejectsTheUser: Story = {
  decorators: [
    withServices(
      fakeUsers({
        create: () =>
          throwError(
            () =>
              new HttpErrorResponse({
                status: 400,
                error: { error: "Ce nom d'utilisateur existe déjà." },
              }),
          ),
      }),
      conflictToasts,
    ),
  ],
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await userEvent.type(await canvas.findByLabelText(/Nom d'utilisateur/), 'alice')
    await userEvent.type(canvas.getByLabelText(/Adresse e-mail/), 'alice@example.com')
    await userEvent.click(canvas.getByRole('button', { name: 'Inviter' }))

    await waitFor(() =>
      expect(conflictToasts.toasts()).toEqual([
        expect.objectContaining({
          variant: 'error',
          message: "Ce nom d'utilisateur existe déjà.",
        }),
      ]),
    )
    expect(args.created).not.toHaveBeenCalled()
    expect(canvas.getByRole('button', { name: 'Inviter' })).toBeEnabled()
    expect(canvas.getByLabelText(/Nom d'utilisateur/)).toHaveValue('alice')
  },
}

const genericFailureToasts = new ToastService()

/** An error with no message from the backend falls back to a generic one. */
export const CreationFailsWithoutDetails: Story = {
  decorators: [
    withServices(
      fakeUsers({ create: () => throwError(() => new Error('network error')) }),
      genericFailureToasts,
    ),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.type(await canvas.findByLabelText(/Nom d'utilisateur/), 'alice')
    await userEvent.type(canvas.getByLabelText(/Adresse e-mail/), 'alice@example.com')
    await userEvent.click(canvas.getByRole('button', { name: 'Inviter' }))

    await waitFor(() =>
      expect(genericFailureToasts.toasts()).toEqual([
        expect.objectContaining({
          variant: 'error',
          message: "Échec de la création de l'utilisateur.",
        }),
      ]),
    )
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
