import type { Meta, StoryObj } from '@storybook/angular-vite'
import { expect, within } from 'storybook/test'
import { LinkMailFailed } from './link-mail-failed'

/** An invitation whose mail did not go out: why, then the activation link to pass on. */
const meta: Meta<LinkMailFailed> = {
  title: 'Shared/LinkMailFailed',
  component: LinkMailFailed,
  args: {
    name: 'dave@example.com',
    mail: {
      email_sent: false,
      email_error: 'email_not_configured',
      activation_url: 'https://app.artiferris.example.com/activate#token=3f9a1c0b7e2d4a6f',
    },
  },
}
export default meta

type Story = StoryObj<LinkMailFailed>

export const NotConfigured: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByText("Le mail n'a pas pu être envoyé")).toBeVisible()
    expect(canvas.getByText(/Transmettez ce lien à dave@example.com/)).toBeVisible()
  },
}

export const SendFailed: Story = {
  args: {
    mail: {
      email_sent: false,
      email_error: 'email_send_failed',
      activation_url: 'https://app.artiferris.example.com/activate#token=3f9a1c0b7e2d4a6f',
    },
  },
}

export const Dismissible: Story = {
  args: { dismissible: true },
}

export const WithoutLink: Story = {
  args: { mail: { email_sent: false, email_error: 'email_send_failed' } },
  play: async ({ canvasElement }) => {
    expect(
      await within(canvasElement).findByText(
        /pour obtenir le lien d'activation de dave@example.com/,
      ),
    ).toBeVisible()
  },
}

export const PasswordReset: Story = {
  args: {
    name: 'bob',
    kind: 'password-reset',
    mail: {
      email_sent: false,
      email_error: 'email_no_address',
      reset_url: 'https://app.artiferris.example.com/reset-password#token=3f9a1c0b7e2d4a6f',
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByText(/il est valable 1 heure/)).toBeVisible()
    expect(canvas.getByText("Ce compte n'a pas d'adresse e-mail vérifiée.")).toBeVisible()
  },
}
