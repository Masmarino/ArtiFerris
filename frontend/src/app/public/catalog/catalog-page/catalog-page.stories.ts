import {
  applicationConfig,
  moduleMetadata,
  type Decorator,
  type Meta,
  type StoryObj,
} from '@storybook/angular-vite'
import { provideRouter } from '@angular/router'
import { expect, fn, within } from 'storybook/test'
import { of } from 'rxjs'
import { AuthService } from '../../../auth/application/auth.service'
import { CatalogService } from '../application/catalog.service'
import { CatalogFormat, CatalogQuery } from '../domain/catalog.entity'
import { catalogEntry, dockerEntry, searchResult } from '../testing/catalog.fixtures'
import { fakeUrlProviders } from '../testing/fake-url'
import { CatalogPage } from './catalog-page'

const search = fn((query: CatalogQuery) =>
  of(searchResult(query.format === 'docker' ? [dockerEntry()] : [catalogEntry()])),
)

function catalog(name: string, format: CatalogFormat): Decorator[] {
  return [
    applicationConfig({
      providers: fakeUrlProviders({}, { catalogName: name, format }),
    }),
    moduleMetadata({ providers: [{ provide: CatalogService, useValue: { search } }] }),
  ]
}

const meta: Meta<CatalogPage> = {
  title: 'Public/Catalog/CatalogPage',
  component: CatalogPage,
  decorators: [
    applicationConfig({
      providers: [
        provideRouter([]),
        { provide: AuthService, useValue: { isAuthenticated: () => false } },
      ],
    }),
  ],
}
export default meta

type Story = StoryObj<CatalogPage>

export const NpmCatalog: Story = {
  decorators: catalog('artiferris-npm', 'npm'),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByRole('heading', { level: 1 })).toHaveTextContent('artiferris-npm')
    expect(canvas.getByText(/Tous les paquets npm publics/)).toBeInTheDocument()
    expect(await canvas.findByRole('link', { name: 'hangar-demo' })).toBeInTheDocument()
    expect(canvas.queryByRole('radiogroup', { name: 'Format' })).toBeNull()
    expect(search).toHaveBeenLastCalledWith(expect.objectContaining({ format: 'npm' }))
  },
}

export const DockerCatalog: Story = {
  decorators: catalog('artiferris-docker', 'docker'),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByRole('heading', { level: 1 })).toHaveTextContent('artiferris-docker')
    expect(canvas.getByText(/images Docker publiques/)).toBeInTheDocument()
    expect(await canvas.findByRole('link', { name: 'team/api' })).toBeInTheDocument()
    expect(search).toHaveBeenLastCalledWith(expect.objectContaining({ format: 'docker' }))
  },
}
