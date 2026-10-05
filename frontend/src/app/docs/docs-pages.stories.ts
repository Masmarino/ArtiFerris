import { applicationConfig, type Meta, type StoryObj } from '@storybook/angular-vite'
import { provideHttpClient } from '@angular/common/http'
import { provideLocationMocks } from '@angular/common/testing'
import { Component, inject, provideAppInitializer } from '@angular/core'
import { Router, RouterOutlet, provideRouter } from '@angular/router'
import { docsRoutes } from '@masmarino/gabarit/docs'
import { expect, waitFor, within } from 'storybook/test'
import { of } from 'rxjs'
import { AuthService } from '../auth/application/auth.service'
import { CatalogService } from '../public/catalog/application/catalog.service'
import { PublicDocsPage } from './public-docs-page'

@Component({
  selector: 'app-story-outlet',
  standalone: true,
  imports: [RouterOutlet],
  template: '<router-outlet />',
})
class StoryOutlet {}

/** The real pages of `public/docs/`, which Storybook serves at the app's paths. */
function readingAt(url: string) {
  return applicationConfig({
    providers: [
      provideHttpClient(),
      provideRouter([
        { path: 'docs', children: docsRoutes(() => Promise.resolve(PublicDocsPage)) },
        { path: '**', children: [] },
      ]),
      provideLocationMocks(),
      provideAppInitializer(() => inject(Router).navigateByUrl(url)),
      { provide: AuthService, useValue: { isAuthenticated: () => false } },
      { provide: CatalogService, useValue: { suggest: () => of([]) } },
    ],
  })
}

const meta: Meta<StoryOutlet> = {
  title: 'Docs/Documentation',
  component: StoryOutlet,
  parameters: { layout: 'fullscreen' },
}
export default meta

type Story = StoryObj<StoryOutlet>

export const PublicPage: Story = {
  decorators: [readingAt('/docs/utilisation/npm')],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(
      () => expect(canvas.getByRole('heading', { level: 1, name: 'Registre npm' })).toBeVisible(),
      { timeout: 5000 },
    )
    const bar = within(canvasElement.querySelector<HTMLElement>('.public-layout__header')!)
    expect(bar.getByRole('link', { name: 'Documentation' })).toHaveAttribute('aria-current', 'page')
    expect(canvas.getByRole('navigation', { name: 'Fil d’Ariane' })).toBeVisible()
  },
}

export const WithTables: Story = {
  decorators: [readingAt('/docs/administration/configuration')],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(
      () => expect(canvas.getByRole('heading', { level: 1, name: 'Configuration' })).toBeVisible(),
      { timeout: 5000 },
    )
  },
}

export const PageNotFound: Story = {
  decorators: [readingAt('/docs/utilisation/absente')],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByText('Page introuvable')).toBeVisible(), {
      timeout: 5000,
    })
  },
}
