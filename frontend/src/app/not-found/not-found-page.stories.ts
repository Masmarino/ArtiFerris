import { applicationConfig, type Meta, type StoryObj } from '@storybook/angular-vite'
import { provideRouter } from '@angular/router'
import { expect, within } from 'storybook/test'
import { AuthService } from '../auth/application/auth.service'
import { NO_SUGGESTIONS } from '../public/catalog/testing/no-suggestions'
import { NotFoundPage } from './not-found-page'

const meta: Meta<NotFoundPage> = {
  title: 'App/NotFoundPage',
  component: NotFoundPage,
  decorators: [
    applicationConfig({
      providers: [
        provideRouter([]),
        NO_SUGGESTIONS,
        { provide: AuthService, useValue: { isAuthenticated: () => false } },
      ],
    }),
  ],
}
export default meta

type Story = StoryObj<NotFoundPage>

export const Default: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByRole('heading', { level: 1 })).toHaveTextContent('Page introuvable')
    expect(canvas.getByRole('link', { name: /Retour à l'accueil/ })).toHaveAttribute(
      'href',
      expect.stringMatching(/\/$/),
    )
  },
}
