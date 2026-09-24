import {
  applicationConfig,
  moduleMetadata,
  type Meta,
  type StoryObj,
} from '@storybook/angular-vite'
import { provideRouter } from '@angular/router'
import { userEvent, waitFor, expect, fn, within } from 'storybook/test'
import { HttpErrorResponse } from '@angular/common/http'
import { NEVER, of, throwError } from 'rxjs'
import { MyRepositoryPage } from './my-repository-page'
import { PersonalRepositoryService } from '../application/personal-repository.service'
import { RepositoriesService } from '../application/repositories.service'
import { OrganizationsService } from '../../admin/application/organizations.service'
import { MeService } from '../../shell/application/me.service'
import { ToastService } from '../../shared/toast.service'
import type { RepositorySummary } from '../domain/repository.entity'

const MY_PROJECT: RepositorySummary = {
  id: 'repo-1',
  name: 'my-project',
  format: 'npm',
  repo_type: 'hosted',
  remote_url: null,
  remote_credentials_set: false,
  group_members: [],
  quota_bytes: null,
  retention_keep_last_n: null,
  is_public: false,
  my_role: 'admin',
  organization_id: 'org-1',
  owner_name: 'florian',
  owner_is_personal: true,
}

function fakePersonalRepositoryService(
  overrides: Partial<PersonalRepositoryService> = {},
): Partial<PersonalRepositoryService> {
  return {
    hasReservedNamespace: () => of(false),
    reserve: () => of(undefined),
    listMyProjects: () => of([MY_PROJECT]),
    ...overrides,
  }
}

const meta: Meta<MyRepositoryPage> = {
  title: 'Repositories/MyRepositoryPage',
  component: MyRepositoryPage,
  decorators: [
    applicationConfig({ providers: [provideRouter([])] }),
    moduleMetadata({
      providers: [
        { provide: PersonalRepositoryService, useValue: fakePersonalRepositoryService() },
        { provide: RepositoriesService, useValue: { list: () => of([]) } },
        { provide: OrganizationsService, useValue: { list: () => of([]) } },
        { provide: MeService, useValue: { isSuperAdmin: () => false } },
      ],
    }),
  ],
}
export default meta

type Story = StoryObj<MyRepositoryPage>

/** The caller has not reserved their personal namespace yet — offers to create it. */
export const NotYetReserved: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByText('Aucun dépôt personnel')).toBeInTheDocument()
    expect(canvas.getByRole('button', { name: 'Créer son dépôt utilisateur' })).toBeInTheDocument()
  },
}

/** Clicking the create button opens the confirmation modal explaining what reserving does. */
export const OpeningTheConfirmModal: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(
      await canvas.findByRole('button', { name: 'Créer son dépôt utilisateur' }),
    )
    await waitFor(() =>
      expect(
        canvas.getByRole('heading', { name: 'Créer son dépôt utilisateur' }),
      ).toBeInTheDocument(),
    )
  },
}

/** Confirming reserves the namespace, then shows the caller's own (now empty) project list. */
export const ReservingTheNamespace: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(
      await canvas.findByRole('button', { name: 'Créer son dépôt utilisateur' }),
    )
    await userEvent.click(await canvas.findByRole('button', { name: 'Créer' }))
    await waitFor(() => expect(canvas.getByText('my-project')).toBeInTheDocument())
  },
}

/** The caller already has a personal namespace — their own projects are shown directly. */
export const AlreadyReserved: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: PersonalRepositoryService,
          useValue: fakePersonalRepositoryService({ hasReservedNamespace: () => of(true) }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByText('my-project')).toBeInTheDocument())
    expect(
      canvas.queryByRole('button', { name: 'Créer son dépôt utilisateur' }),
    ).not.toBeInTheDocument()
  },
}

function withFakes(overrides: Partial<PersonalRepositoryService>) {
  const toast = { success: fn(), error: fn() }
  const personal = fakePersonalRepositoryService(overrides)
  const decorator = moduleMetadata({
    providers: [
      { provide: PersonalRepositoryService, useValue: personal },
      { provide: ToastService, useValue: toast },
    ],
  })
  return { toast, personal, decorator }
}
async function openConfirmModal(canvas: ReturnType<typeof within>) {
  await userEvent.click(await canvas.findByRole('button', { name: 'Créer son dépôt utilisateur' }))
  await canvas.findByRole('button', { name: 'Créer' })
}

const loading = withFakes({ hasReservedNamespace: () => NEVER })
export const Loading: Story = {
  decorators: [loading.decorator],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByRole('status')).toHaveTextContent('Chargement…')
    expect(
      canvas.queryByRole('button', { name: 'Créer son dépôt utilisateur' }),
    ).not.toBeInTheDocument()
  },
}

const checkFailed = withFakes({ hasReservedNamespace: () => throwError(() => new Error('down')) })
/** If the check fails the caller is treated as not reserved yet, and told so. */
export const CheckingReservationFailed: Story = {
  decorators: [checkFailed.decorator],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(
      await canvas.findByRole('button', { name: 'Créer son dépôt utilisateur' }),
    ).toBeInTheDocument()
    expect(checkFailed.toast.error).toHaveBeenCalledWith(
      'Échec de la vérification de votre dépôt personnel.',
    )
  },
}

const reserving = withFakes({ reserve: fn(() => NEVER) })
/** While the reservation request is pending, the confirm button is busy. */
export const Reserving: Story = {
  decorators: [reserving.decorator],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await openConfirmModal(canvas)
    await userEvent.click(canvas.getByRole('button', { name: 'Créer' }))
    const busy = await canvas.findByRole('button', { name: /Créer$/ })
    await waitFor(() => expect(busy).toBeDisabled())
    expect(busy).toHaveAttribute('aria-busy', 'true')
    expect(reserving.personal.reserve).toHaveBeenCalledOnce()
  },
}

const reserveFailed = withFakes({
  reserve: fn(() =>
    throwError(
      () => new HttpErrorResponse({ status: 400, error: { error: 'quota de dépôts atteint' } }),
    ),
  ),
})
/** A failed reservation keeps the modal open and shows the server's own message. */
export const ReservationFailed: Story = {
  decorators: [reserveFailed.decorator],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await openConfirmModal(canvas)
    await userEvent.click(canvas.getByRole('button', { name: 'Créer' }))
    await waitFor(() =>
      expect(reserveFailed.toast.error).toHaveBeenCalledWith('quota de dépôts atteint'),
    )
    expect(canvas.getByRole('heading', { name: 'Créer son dépôt utilisateur' })).toBeInTheDocument()
    expect(canvas.getByRole('button', { name: 'Créer' })).toBeEnabled()
  },
}

const reserveFailedGeneric = withFakes({
  reserve: fn(() => throwError(() => new Error('network error'))),
})
export const ReservationFailedWithoutServerMessage: Story = {
  decorators: [reserveFailedGeneric.decorator],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await openConfirmModal(canvas)
    await userEvent.click(canvas.getByRole('button', { name: 'Créer' }))
    await waitFor(() =>
      expect(reserveFailedGeneric.toast.error).toHaveBeenCalledWith(
        'Échec de la création de votre dépôt utilisateur.',
      ),
    )
  },
}

const alreadyReservedElsewhere = withFakes({
  reserve: fn(() => throwError(() => new HttpErrorResponse({ status: 409 }))),
})
/** A 409 means the namespace already exists (e.g. another tab), which counts as success. */
export const ReservationConflict: Story = {
  decorators: [alreadyReservedElsewhere.decorator],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await openConfirmModal(canvas)
    await userEvent.click(canvas.getByRole('button', { name: 'Créer' }))
    await waitFor(() => expect(canvas.getByText('my-project')).toBeInTheDocument())
    expect(alreadyReservedElsewhere.toast.error).not.toHaveBeenCalled()
    expect(
      canvas.queryByRole('heading', { name: 'Créer son dépôt utilisateur' }),
    ).not.toBeInTheDocument()
  },
}

const cancelling = withFakes({ reserve: fn(() => of(undefined)) })
export const CancellingTheConfirmModal: Story = {
  decorators: [cancelling.decorator],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await openConfirmModal(canvas)
    await userEvent.click(canvas.getByRole('button', { name: 'Fermer' }))
    await waitFor(() =>
      expect(
        canvas.queryByRole('heading', { name: 'Créer son dépôt utilisateur' }),
      ).not.toBeInTheDocument(),
    )
    expect(cancelling.personal.reserve).not.toHaveBeenCalled()
    expect(canvas.getByRole('button', { name: 'Créer son dépôt utilisateur' })).toBeInTheDocument()
  },
}
