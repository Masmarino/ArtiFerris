import {
  applicationConfig,
  moduleMetadata,
  type Decorator,
  type Meta,
  type StoryObj,
} from '@storybook/angular-vite'
import { provideLocationMocks } from '@angular/common/testing'
import { inject, provideAppInitializer } from '@angular/core'
import { Router, provideRouter } from '@angular/router'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { of } from 'rxjs'
import { AuthService } from '../../auth/application/auth.service'
import { CatalogService } from '../catalog/application/catalog.service'
import { CatalogSuggestion } from '../catalog/domain/catalog.entity'
import { catalogSuggestion, dockerSuggestion } from '../catalog/testing/catalog.fixtures'
import { CurrentUrl } from '../catalog/testing/current-url'
import { PublicLayout } from './public-layout'

const routes = [
  { path: 'explorer', children: [] },
  { path: 'artiferris-npm', children: [] },
  { path: '**', children: [] },
]

function suggesting(suggestions: CatalogSuggestion[] = []): Decorator {
  return moduleMetadata({
    providers: [{ provide: CatalogService, useValue: { suggest: () => of(suggestions) } }],
  })
}

const meta: Meta<PublicLayout> = {
  title: 'Public/PublicLayout',
  component: PublicLayout,
  decorators: [
    applicationConfig({
      providers: [
        provideRouter(routes),
        provideLocationMocks(),
        { provide: AuthService, useValue: { isAuthenticated: () => false } },
      ],
    }),
    moduleMetadata({ imports: [CurrentUrl] }),
  ],
  render: () => ({
    template:
      '<app-public-layout><p>Contenu de la page</p><app-story-current-url /></app-public-layout>',
  }),
}
export default meta

type Story = StoryObj<PublicLayout>

/** Logo, quick search and login link in the header, the page's own content projected below. */
export const Default: Story = {
  decorators: [suggesting()],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(canvas.getByRole('img', { name: 'ArtiFerris logo' })).toBeInTheDocument()
    expect(canvas.getByRole('link', { name: 'Explorer les paquets publics' })).toHaveAttribute(
      'href',
      expect.stringMatching(/\/explorer$/),
    )
    expect(canvas.getByRole('link', { name: 'Se connecter' })).toHaveAttribute(
      'href',
      expect.stringMatching(/\/login$/),
    )
    expect(canvas.getByRole('combobox', { name: 'Rechercher un paquet' })).toBeInTheDocument()
    expect(within(canvas.getByRole('main')).getByText('Contenu de la page')).toBeInTheDocument()
  },
}

/** A signed-in visitor gets a way back to their dashboard instead of "Se connecter". */
export const SignedIn: Story = {
  decorators: [
    suggesting(),
    moduleMetadata({
      providers: [{ provide: AuthService, useValue: { isAuthenticated: () => true } }],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(canvas.getByRole('link', { name: 'Mes dépôts' })).toHaveAttribute(
      'href',
      expect.stringMatching(/\/repositories$/),
    )
    expect(canvas.queryByRole('link', { name: 'Se connecter' })).toBeNull()
  },
}

/** Submitting the header search opens the explorer with the text. */
export const HeaderSearch: Story = {
  decorators: [suggesting()],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)

    await userEvent.type(
      canvas.getByRole('combobox', { name: 'Rechercher un paquet' }),
      'demo{Enter}',
    )

    await waitFor(() =>
      expect(canvas.getByLabelText('URL courante')).toHaveTextContent('/explorer?q=demo'),
    )
  },
}

/** On a catalog page the header search stays on that catalog. */
export const HeaderSearchOnCatalogPage: Story = {
  decorators: [
    suggesting(),
    applicationConfig({
      providers: [provideAppInitializer(() => inject(Router).navigateByUrl('/artiferris-npm'))],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() =>
      expect(canvas.getByLabelText('URL courante')).toHaveTextContent('/artiferris-npm'),
    )

    await userEvent.type(
      canvas.getByRole('combobox', { name: 'Rechercher un paquet' }),
      'left{Enter}',
    )

    await waitFor(() =>
      expect(canvas.getByLabelText('URL courante')).toHaveTextContent('/artiferris-npm?q=left'),
    )
  },
}

/** Typing in the header search suggests packages, and picking one opens its page. */
export const HeaderSuggestions: Story = {
  decorators: [
    suggesting([catalogSuggestion({ name: 'left-pad' }), dockerSuggestion({ name: 'left-proxy' })]),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const input = canvas.getByRole('combobox', { name: 'Rechercher un paquet' })

    await userEvent.type(input, 'left')

    expect(await canvas.findAllByRole('option')).toHaveLength(2)
    expect(input).toHaveAttribute('aria-expanded', 'true')

    await userEvent.keyboard('{ArrowDown}{ArrowDown}{Enter}')

    await waitFor(() =>
      expect(canvas.getByLabelText('URL courante')).toHaveTextContent(
        '/o/acme/images/packages/docker/left-proxy',
      ),
    )
  },
}
