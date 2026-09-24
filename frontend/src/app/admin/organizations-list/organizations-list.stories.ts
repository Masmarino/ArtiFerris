import { moduleMetadata, type Meta, type StoryObj } from '@storybook/angular-vite'
import { Router } from '@angular/router'
import { expect, fn, userEvent, waitFor, within } from 'storybook/test'
import { NEVER, of, throwError } from 'rxjs'
import { OrganizationsList } from './organizations-list'
import { OrganizationsService } from '../application/organizations.service'
import { ToastService } from '../../shared/toast.service'
import type { OrganizationSummary } from '../domain/organization.entity'

const ORGS: OrganizationSummary[] = [
  { id: 'org-public', slug: 'public', display_name: 'Public', is_public: true },
  { id: 'org-acme', slug: 'acme', display_name: 'Acme Corp', is_public: false },
  { id: 'org-globex', slug: 'globex', display_name: 'Globex', is_public: false },
]

function fakeOrgs(overrides: Partial<OrganizationsService> = {}): Partial<OrganizationsService> {
  return {
    list: fn(() => of(ORGS)),
    create: fn(() => of(ORGS[1])),
    ...overrides,
  }
}

const router = { navigate: fn(() => Promise.resolve(true)) }
const toast = { success: fn(), error: fn() }

function withOrgs(orgs: Partial<OrganizationsService>) {
  return moduleMetadata({ providers: [{ provide: OrganizationsService, useValue: orgs }] })
}

const meta: Meta<OrganizationsList> = {
  title: 'Admin/OrganizationsList',
  component: OrganizationsList,
  beforeEach: () => {
    router.navigate.mockClear()
    toast.success.mockClear()
    toast.error.mockClear()
  },
  decorators: [
    moduleMetadata({
      providers: [
        // A real Router would try to match the Storybook iframe's own URL against an empty route table.
        { provide: Router, useValue: router },
        { provide: OrganizationsService, useValue: fakeOrgs() },
        { provide: ToastService, useValue: toast },
      ],
    }),
  ],
}
export default meta

type Story = StoryObj<OrganizationsList>

const listedOrgs = fakeOrgs()
export const Default: Story = {
  decorators: [withOrgs(listedOrgs)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByText('Acme Corp')).toBeInTheDocument())
    expect(canvas.getByText('globex')).toBeInTheDocument()
    expect(canvas.getByText('Public')).toBeInTheDocument()
    expect(canvas.getByRole('button', { name: 'Nouvelle organisation' })).toBeInTheDocument()
    // Always bypasses the service's cache, so a just-created organization shows up.
    expect(listedOrgs.list).toHaveBeenCalledWith({ forceRefresh: true })
  },
}

export const Loading: Story = {
  decorators: [withOrgs(fakeOrgs({ list: () => NEVER }))],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByRole('status')).toHaveTextContent('Chargement…')
    expect(canvas.queryByRole('table')).not.toBeInTheDocument()
  },
}

export const Empty: Story = {
  decorators: [withOrgs(fakeOrgs({ list: () => of([]) }))],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByText('Aucune organisation')).toBeInTheDocument()
    expect(canvas.queryByRole('table')).not.toBeInTheDocument()
    // One in the page header, one in the empty state.
    expect(canvas.getAllByRole('button', { name: 'Nouvelle organisation' })).toHaveLength(2)
  },
}

export const LoadFailed: Story = {
  decorators: [withOrgs(fakeOrgs({ list: () => throwError(() => new Error('network error')) }))],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByRole('alert')).toHaveTextContent(
      'Échec du chargement des organisations.',
    )
    expect(canvas.queryByRole('table')).not.toBeInTheDocument()
  },
}

/** Clicking a row opens that organization's detail page. */
export const OpeningAnOrganization: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByText('Acme Corp')).toBeInTheDocument())
    await userEvent.click(canvas.getByText('Acme Corp'))

    expect(router.navigate).toHaveBeenCalledWith(['/admin/organizations', 'org-acme'])
  },
}

export const CreateOrganizationModalOpen: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('button', { name: 'Nouvelle organisation' }))

    expect(
      await canvas.findByRole('heading', { name: 'Nouvelle organisation' }),
    ).toBeInTheDocument()
    expect(canvas.getByLabelText(/^Sous-domaine/)).toBeInTheDocument()
  },
}

/** Cancelling the modal closes it without reloading the list. */
export const CreateModalCancelled: Story = {
  decorators: [withOrgs(fakeOrgs())],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('button', { name: 'Nouvelle organisation' }))
    await userEvent.click(await canvas.findByRole('button', { name: 'Fermer' }))

    await waitFor(() =>
      expect(
        canvas.queryByRole('heading', { name: 'Nouvelle organisation' }),
      ).not.toBeInTheDocument(),
    )
  },
}

// Stateful so the reload after creating returns the new organization.
let acmeCreated = false
const creatingOrgs = fakeOrgs({
  list: fn(() => of(acmeCreated ? ORGS : ORGS.filter((o) => o.slug !== 'acme'))),
  create: fn(() => {
    acmeCreated = true
    return of(ORGS[1])
  }),
})
/** Creating an organization closes the modal and reloads the list with the new row in it. */
export const CreatingAnOrganization: Story = {
  beforeEach: () => {
    acmeCreated = false
  },
  decorators: [withOrgs(creatingOrgs)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByText('Globex')).toBeInTheDocument())
    expect(canvas.queryByText('Acme Corp')).not.toBeInTheDocument()

    await userEvent.click(canvas.getByRole('button', { name: 'Nouvelle organisation' }))
    await userEvent.type(await canvas.findByLabelText(/^Sous-domaine/), 'acme')
    await userEvent.type(canvas.getByLabelText(/^Nom/), 'Acme Corp')
    await userEvent.click(canvas.getByRole('button', { name: 'Créer' }))

    await waitFor(() => expect(creatingOrgs.create).toHaveBeenCalledWith('acme', 'Acme Corp'))
    await waitFor(() => expect(canvas.getByText('Acme Corp')).toBeInTheDocument())
    expect(canvas.queryByRole('heading', { name: 'Nouvelle organisation' })).not.toBeInTheDocument()
    expect(creatingOrgs.list).toHaveBeenCalledTimes(2)
  },
}
