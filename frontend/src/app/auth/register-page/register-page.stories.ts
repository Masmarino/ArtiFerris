import { applicationConfig, type Meta, type StoryObj } from '@storybook/angular-vite'
import { provideRouter } from '@angular/router'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { of } from 'rxjs'
import { RegisterPage } from './register-page'
import { AuthPort } from '../application/auth.port'
import { fakeAuthPort, refused, withAuthPort } from '../kit/auth-story-helpers'

const page = (port: AuthPort) =>
  applicationConfig({ providers: [provideRouter([]), withAuthPort(port)] })

async function register(canvasElement: HTMLElement) {
  const canvas = within(canvasElement)
  await userEvent.type(await canvas.findByLabelText("Nom d'utilisateur"), 'marie')
  await userEvent.type(canvas.getByLabelText('Adresse e-mail'), 'marie@example.com')
  await userEvent.type(canvas.getByLabelText('Mot de passe'), 'sup3r-s3cret!')
  await userEvent.click(canvas.getByRole('button', { name: 'Créer mon compte' }))
  return canvas
}

/** Open registration: the new account is signed in, then sets up its second factor. */
const meta: Meta<RegisterPage> = {
  title: 'Auth/RegisterPage',
  component: RegisterPage,
  parameters: { layout: 'fullscreen' },
  decorators: [page(fakeAuthPort())],
}
export default meta

type Story = StoryObj<RegisterPage>

export const Default: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByLabelText("Nom d'utilisateur")).toHaveFocus())
  },
}

export const AfterSubmitEntersMfaSetup: Story = {
  play: async ({ canvasElement }) => {
    const canvas = await register(canvasElement)
    await expect(
      await canvas.findByRole('heading', { name: 'Protégez votre compte' }),
    ).toBeVisible()
  },
}

export const UsernameAlreadyTaken: Story = {
  decorators: [
    page(
      fakeAuthPort({
        register: () => refused(400, 'username already taken', 'username_taken'),
      }),
    ),
  ],
  play: async ({ canvasElement }) => {
    const canvas = await register(canvasElement)
    await expect(
      await canvas.findByText("Ce nom d'utilisateur ou cette adresse e-mail est déjà utilisé"),
    ).toBeVisible()
    await waitFor(() => expect(canvas.getByLabelText("Nom d'utilisateur")).toHaveFocus())
  },
}

export const RegistrationClosed: Story = {
  decorators: [
    page(fakeAuthPort({ getSsoConfig: () => of({ type: null, registration_enabled: false }) })),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(
      await canvas.findByRole('heading', { name: 'Les inscriptions sont fermées' }),
    ).toBeVisible()
  },
}
