import { HttpErrorResponse } from '@angular/common/http'
import { moduleMetadata, type Meta, type StoryObj } from '@storybook/angular-vite'
import { expect, fn, userEvent, waitFor, within } from 'storybook/test'
import { NEVER, of, throwError } from 'rxjs'
import { CreateUserModal } from './create-user-modal'
import { UsersService } from '../application/users.service'
import type { InvitedUser } from '../domain/user.entity'

const CREATED: InvitedUser = {
  id: 'u9',
  username: 'alice',
  is_super_admin: false,
  organization_id: 'org-acme',
  email: 'alice@example.com',
  invitation_pending: true,
  created_at: '2026-10-06T08:00:00Z',
  invitation_expires_at: '2026-10-07T08:00:00Z',
  mfa_enabled: false,
  email_sent: true,
}

function fakeUsers(overrides: Partial<UsersService> = {}): Partial<UsersService> {
  return { create: fn(() => of(CREATED)), ...overrides }
}

function withServices(users: Partial<UsersService>) {
  return moduleMetadata({ providers: [{ provide: UsersService, useValue: users }] })
}

const meta: Meta<CreateUserModal> = {
  title: 'Users/CreateUserModal',
  component: CreateUserModal,
  decorators: [withServices(fakeUsers())],
  args: { invited: fn(), closed: fn() },
}
export default meta

type Story = StoryObj<CreateUserModal>

const emailField = (canvas: ReturnType<typeof within>) => canvas.findByLabelText('Adresse e-mail')
const sendButton = (canvas: ReturnType<typeof within>) =>
  canvas.getByRole('button', { name: "Envoyer l'invitation" })

export const Default: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByRole('heading', { name: 'Inviter un utilisateur' })).toBeVisible()
    expect(canvas.getByRole('switch', { name: 'Super-administrateur' })).not.toBeChecked()
  },
}

export const Errors: Story = {
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await userEvent.type(await emailField(canvas), 'alice@example')
    await userEvent.click(sendButton(canvas))
    expect(
      await canvas.findByText('Saisissez une adresse e-mail valide, par exemple nom@exemple.fr'),
    ).toBeVisible()
    expect(args.invited).not.toHaveBeenCalled()
  },
}

export const Sent: Story = {
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await userEvent.type(await emailField(canvas), 'alice@example.com')
    await userEvent.click(canvas.getByRole('switch', { name: 'Super-administrateur' }))
    await userEvent.click(sendButton(canvas))

    expect(await canvas.findByText('Invitation envoyée à alice@example.com')).toBeVisible()
    expect(args.invited).toHaveBeenCalledWith(CREATED)
    await userEvent.click(canvas.getByRole('button', { name: 'Terminé' }))
    expect(args.closed).toHaveBeenCalledTimes(1)
  },
}

export const MailFailed: Story = {
  decorators: [
    withServices(
      fakeUsers({
        create: fn(() =>
          of({
            ...CREATED,
            email_sent: false,
            email_error: 'email_send_failed' as const,
            activation_url: 'https://app.artiferris.example.com/activate#token=3f9a1c',
          }),
        ),
      }),
    ),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.type(await emailField(canvas), 'alice@example.com')
    await userEvent.click(sendButton(canvas))
    expect(await canvas.findByText("Le mail n'a pas pu être envoyé")).toBeVisible()
    expect(canvas.getByText("Le serveur mail n'a pas accepté le message.")).toBeVisible()
  },
}

export const Sending: Story = {
  decorators: [withServices(fakeUsers({ create: () => NEVER }))],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.type(await emailField(canvas), 'alice@example.com')
    await userEvent.click(sendButton(canvas))
    await waitFor(() => expect(canvas.getByRole('button', { name: 'Annuler' })).toBeDisabled())
  },
}

export const Refused: Story = {
  decorators: [
    withServices(
      fakeUsers({
        create: () =>
          throwError(
            () =>
              new HttpErrorResponse({
                status: 409,
                error: { error: 'email already in use', code: 'email_taken' },
              }),
          ),
      }),
    ),
  ],
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await userEvent.type(await emailField(canvas), 'alice@example.com')
    await userEvent.click(sendButton(canvas))
    expect(
      await canvas.findByText('Cette adresse e-mail est déjà utilisée par un compte'),
    ).toBeVisible()
    expect(args.invited).not.toHaveBeenCalled()
    expect(canvas.getByLabelText('Adresse e-mail')).toHaveValue('alice@example.com')
  },
}

export const Failed: Story = {
  decorators: [
    withServices(fakeUsers({ create: () => throwError(() => new Error('network error')) })),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.type(await emailField(canvas), 'alice@example.com')
    await userEvent.click(sendButton(canvas))
    expect(
      await canvas.findByText("L'invitation n'a pas pu être envoyée. Réessayez plus tard."),
    ).toBeVisible()
  },
}

export const Cancelling: Story = {
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('button', { name: 'Annuler' }))
    expect(args.closed).toHaveBeenCalledTimes(1)
    expect(args.invited).not.toHaveBeenCalled()
  },
}
