import { applicationConfig, type Meta, type StoryObj } from '@storybook/angular-vite'
import { ActivatedRoute, convertToParamMap, provideRouter } from '@angular/router'
import { expect, fn, userEvent, waitFor, within } from 'storybook/test'
import { of } from 'rxjs'
import { ActivatePage } from './activate-page'
import { AuthPort } from '../application/auth.port'
import { fakeAuthPort, refused, withAuthPort } from '../kit/auth-story-helpers'

const TOKEN = 'ab12'.repeat(16)

const page = (port: AuthPort, fragment: string | null = `token=${TOKEN}`) =>
  applicationConfig({
    providers: [
      provideRouter([]),
      withAuthPort(port),
      {
        provide: ActivatedRoute,
        useValue: { snapshot: { fragment, queryParamMap: convertToParamMap({}) } },
      },
    ],
  })

async function activate(canvasElement: HTMLElement, username = 'marie') {
  const canvas = within(canvasElement)
  await userEvent.type(await canvas.findByLabelText("Nom d'utilisateur"), username)
  await userEvent.type(canvas.getByLabelText('Nouveau mot de passe'), 'sup3r-s3cret!')
  await userEvent.type(canvas.getByLabelText('Confirmez le mot de passe'), 'sup3r-s3cret!')
  await userEvent.click(canvas.getByRole('button', { name: 'Activer mon compte' }))
  return canvas
}

/**
 * Where an invitation mail's link lands. The administrator invited by e-mail only: the invitee
 * chooses their username along with their password.
 */
const meta: Meta<ActivatePage> = {
  title: 'Auth/ActivatePage',
  component: ActivatePage,
  parameters: { layout: 'fullscreen' },
  decorators: [page(fakeAuthPort({ activate: () => of(undefined) }))],
}
export default meta

type Story = StoryObj<ActivatePage>

export const Default: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByLabelText("Nom d'utilisateur")).toHaveFocus())
  },
}

const activateCall = fn(() => of(undefined))

export const Activated: Story = {
  decorators: [page(fakeAuthPort({ activate: activateCall }))],
  play: async ({ canvasElement }) => {
    const canvas = await activate(canvasElement, 'Marie')
    await expect(
      await canvas.findByRole('heading', { name: 'Votre compte est activé' }),
    ).toBeVisible()
    await expect(activateCall).toHaveBeenCalledWith(TOKEN, 'Marie', 'sup3r-s3cret!')
  },
}

export const UsernameTaken: Story = {
  decorators: [
    page(
      fakeAuthPort({
        activate: () => refused(400, 'username already taken', 'username_taken'),
      }),
    ),
  ],
  play: async ({ canvasElement }) => {
    const canvas = await activate(canvasElement)
    await expect(await canvas.findByText("Ce nom d'utilisateur est déjà utilisé")).toBeVisible()
    await waitFor(() => expect(canvas.getByLabelText("Nom d'utilisateur")).toHaveFocus())
  },
}

export const MissingToken: Story = {
  decorators: [page(fakeAuthPort(), null)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(
      await canvas.findByRole('heading', { name: 'Ce lien ne fonctionne pas' }),
    ).toBeVisible()
  },
}

export const ExpiredLink: Story = {
  decorators: [
    page(
      fakeAuthPort({
        activate: () => refused(400, 'invitation expired', 'invitation_expired'),
      }),
    ),
  ],
  play: async ({ canvasElement }) => {
    const canvas = await activate(canvasElement)
    await expect(
      await canvas.findByRole('heading', { name: 'Ce lien ne fonctionne pas' }),
    ).toBeVisible()
  },
}
