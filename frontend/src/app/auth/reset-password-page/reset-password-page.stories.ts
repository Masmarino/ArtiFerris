import { applicationConfig, type Meta, type StoryObj } from '@storybook/angular-vite'
import { ActivatedRoute, convertToParamMap, provideRouter } from '@angular/router'
import { expect, fn, userEvent, waitFor, within } from 'storybook/test'
import { of } from 'rxjs'
import { ResetPasswordPage } from './reset-password-page'
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

async function choose(canvasElement: HTMLElement) {
  const canvas = within(canvasElement)
  await userEvent.type(await canvas.findByLabelText('Nouveau mot de passe'), 'n3w-s3cret!')
  await userEvent.type(canvas.getByLabelText('Confirmez le mot de passe'), 'n3w-s3cret!')
  await userEvent.click(canvas.getByRole('button', { name: 'Enregistrer le mot de passe' }))
  return canvas
}

/** Where the link of an administrator's password reset lands. */
const meta: Meta<ResetPasswordPage> = {
  title: 'Auth/ResetPasswordPage',
  component: ResetPasswordPage,
  parameters: { layout: 'fullscreen' },
  decorators: [page(fakeAuthPort())],
}
export default meta

type Story = StoryObj<ResetPasswordPage>

export const Default: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByLabelText('Nouveau mot de passe')).toHaveFocus())
  },
}

const resetCall = fn(() => of(undefined))

export const Saved: Story = {
  decorators: [page(fakeAuthPort({ resetPassword: resetCall }))],
  play: async ({ canvasElement }) => {
    const canvas = await choose(canvasElement)
    expect(
      await canvas.findByRole('heading', { name: 'Votre mot de passe est enregistré' }),
    ).toBeVisible()
    expect(resetCall).toHaveBeenCalledWith(TOKEN, 'n3w-s3cret!')
  },
}

export const LinkRefused: Story = {
  decorators: [
    page(
      fakeAuthPort({
        resetPassword: () =>
          refused(400, 'invalid or expired password reset link', 'password_reset_link_invalid'),
      }),
    ),
  ],
  play: async ({ canvasElement }) => {
    const canvas = await choose(canvasElement)
    expect(await canvas.findByRole('heading', { name: 'Ce lien ne fonctionne pas' })).toBeVisible()
  },
}

export const DeadLink: Story = {
  decorators: [page(fakeAuthPort(), null)],
  play: async ({ canvasElement }) => {
    expect(
      await within(canvasElement).findByRole('heading', { name: 'Ce lien ne fonctionne pas' }),
    ).toBeVisible()
  },
}
