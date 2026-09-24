import { moduleMetadata, type Meta, type StoryObj } from '@storybook/angular-vite'
import { expect, fn, userEvent, waitFor, within } from 'storybook/test'
import { NEVER, of, throwError } from 'rxjs'
import { OrganizationMembers } from './organization-members'
import { OrganizationMembersService } from '../application/organization-members.service'
import { ConfirmService } from '../../shared/confirm.service'
import { ToastService } from '../../shared/toast.service'
import type { OrganizationMember } from '../domain/organization-member.entity'

const ALICE: OrganizationMember = {
  id: 'u-alice',
  username: 'alice',
  email: 'alice@acme.test',
  is_organization_admin: true,
  invitation_pending: false,
}
const BOB: OrganizationMember = {
  id: 'u-bob',
  username: 'bob',
  email: 'bob@acme.test',
  is_organization_admin: false,
  invitation_pending: false,
}
const CAROL: OrganizationMember = {
  id: 'u-carol',
  username: 'carol',
  email: 'carol@acme.test',
  is_organization_admin: false,
  invitation_pending: true,
}
const MEMBERS = [ALICE, BOB, CAROL]

function fakeMembers(
  overrides: Partial<OrganizationMembersService> = {},
): Partial<OrganizationMembersService> {
  return {
    list: fn(() => of(MEMBERS)),
    invite: fn(() => of(CAROL)),
    setOrganizationAdmin: fn(() => of(undefined)),
    ...overrides,
  }
}

const toast = { success: fn(), error: fn() }

/** Stands in for the confirmation dialog the component opens before changing a role. */
function fakeConfirm(answer = true) {
  return { ask: fn(() => Promise.resolve(answer)) }
}

function withMembers(
  members: Partial<OrganizationMembersService>,
  confirm: ReturnType<typeof fakeConfirm> = fakeConfirm(),
) {
  return moduleMetadata({
    providers: [
      { provide: OrganizationMembersService, useValue: members },
      { provide: ConfirmService, useValue: confirm },
    ],
  })
}

function memberRow(canvas: ReturnType<typeof within>, username: string) {
  return within(canvas.getByRole('row', { name: new RegExp(username) }))
}

async function openInviteForm(canvas: ReturnType<typeof within>) {
  await userEvent.click(await canvas.findByRole('button', { name: 'Inviter un membre' }))
}

const meta: Meta<OrganizationMembers> = {
  title: 'Admin/OrganizationMembers',
  component: OrganizationMembers,
  args: { organizationId: 'org-acme' },
  beforeEach: () => {
    toast.success.mockClear()
    toast.error.mockClear()
  },
  decorators: [
    moduleMetadata({
      providers: [
        { provide: OrganizationMembersService, useValue: fakeMembers() },
        { provide: ToastService, useValue: toast },
      ],
    }),
  ],
}
export default meta

type Story = StoryObj<OrganizationMembers>

const listedMembers = fakeMembers()
/** An admin, a regular member and a pending invitation, each with the matching role action. */
export const Populated: Story = {
  decorators: [withMembers(listedMembers)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByText('alice')).toBeInTheDocument())
    expect(listedMembers.list).toHaveBeenCalledWith('org-acme')
    expect(canvas.getByRole('table', { name: "Membres de l'organisation" })).toBeInTheDocument()
    expect(
      memberRow(canvas, 'alice').getByRole('button', { name: 'Rétrograder' }),
    ).toBeInTheDocument()
    expect(memberRow(canvas, 'bob').getByRole('button', { name: 'Promouvoir' })).toBeInTheDocument()
    expect(memberRow(canvas, 'carol').getAllByRole('cell')[3]).toHaveTextContent('Oui')
    expect(memberRow(canvas, 'bob').getAllByRole('cell')[3]).toHaveTextContent('Non')
    expect(canvas.getByRole('button', { name: 'Inviter un membre' })).toBeInTheDocument()
  },
}

export const Loading: Story = {
  decorators: [withMembers(fakeMembers({ list: () => NEVER }))],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByRole('status')).toHaveTextContent('Chargement…')
    expect(canvas.queryByRole('table')).not.toBeInTheDocument()
  },
}

export const NoMembers: Story = {
  decorators: [withMembers(fakeMembers({ list: () => of([]) }))],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByText('Aucun membre')).toBeInTheDocument())
    expect(canvas.getByRole('button', { name: 'Inviter un membre' })).toBeInTheDocument()
  },
}

export const LoadFailed: Story = {
  decorators: [withMembers(fakeMembers({ list: () => throwError(() => new Error('boom')) }))],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByRole('alert')).toHaveTextContent('Échec du chargement des membres.')
    expect(canvas.queryByRole('table')).not.toBeInTheDocument()
  },
}

/** "Inviter" stays disabled until both a username and an e-mail are typed. */
export const InviteFormNeedsBothFields: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await openInviteForm(canvas)
    expect(canvas.queryByRole('button', { name: 'Inviter un membre' })).not.toBeInTheDocument()
    expect(canvas.getByRole('button', { name: 'Inviter' })).toBeDisabled()

    await userEvent.type(canvas.getByLabelText("Nom d'utilisateur"), 'dave')
    expect(canvas.getByRole('button', { name: 'Inviter' })).toBeDisabled()

    await userEvent.type(canvas.getByLabelText('Adresse e-mail'), 'dave@acme.test')
    await waitFor(() => expect(canvas.getByRole('button', { name: 'Inviter' })).toBeEnabled())
  },
}

export const CancellingTheInvite: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await openInviteForm(canvas)
    await userEvent.type(canvas.getByLabelText("Nom d'utilisateur"), 'dave')
    await userEvent.click(canvas.getByRole('button', { name: 'Annuler' }))

    expect(canvas.getByRole('button', { name: 'Inviter un membre' })).toBeInTheDocument()
    expect(canvas.queryByLabelText("Nom d'utilisateur")).not.toBeInTheDocument()
  },
}

const inviting = fakeMembers()
export const InvitingAMember: Story = {
  decorators: [withMembers(inviting)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByText('alice')).toBeInTheDocument())
    await openInviteForm(canvas)
    await userEvent.type(canvas.getByLabelText("Nom d'utilisateur"), 'dave')
    await userEvent.type(canvas.getByLabelText('Adresse e-mail'), 'dave@acme.test')
    await userEvent.click(canvas.getByLabelText('Administrateur de cette organisation'))
    await userEvent.click(canvas.getByRole('button', { name: 'Inviter' }))

    await waitFor(() =>
      expect(inviting.invite).toHaveBeenCalledWith('org-acme', 'dave', 'dave@acme.test', true),
    )
    expect(toast.success).toHaveBeenCalledWith('dave a été invité·e.')
    // The form closes and the member list is fetched again.
    await waitFor(() =>
      expect(canvas.getByRole('button', { name: 'Inviter un membre' })).toBeInTheDocument(),
    )
    expect(inviting.list).toHaveBeenCalledTimes(2)
  },
}

const inviteInFlight = fakeMembers({ invite: fn(() => NEVER) })
export const InviteInFlight: Story = {
  decorators: [withMembers(inviteInFlight)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await openInviteForm(canvas)
    await userEvent.type(canvas.getByLabelText("Nom d'utilisateur"), 'dave')
    await userEvent.type(canvas.getByLabelText('Adresse e-mail'), 'dave@acme.test')
    await userEvent.click(canvas.getByRole('button', { name: 'Inviter' }))

    const button = await canvas.findByRole('button', { name: /Inviter/ })
    await waitFor(() => expect(button).toHaveAttribute('aria-busy', 'true'))
    expect(button).toBeDisabled()
    expect(inviteInFlight.invite).toHaveBeenCalledTimes(1)
  },
}

export const InviteFailed: Story = {
  decorators: [withMembers(fakeMembers({ invite: fn(() => throwError(() => new Error('boom'))) }))],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await openInviteForm(canvas)
    await userEvent.type(canvas.getByLabelText("Nom d'utilisateur"), 'dave')
    await userEvent.type(canvas.getByLabelText('Adresse e-mail'), 'dave@acme.test')
    await userEvent.click(canvas.getByRole('button', { name: 'Inviter' }))

    await waitFor(() => expect(toast.error).toHaveBeenCalledWith("Échec de l'invitation."))
    expect(toast.success).not.toHaveBeenCalled()
    // The form stays open with what was typed, so the invite can be retried.
    expect(canvas.getByLabelText("Nom d'utilisateur")).toHaveValue('dave')
    expect(canvas.getByRole('button', { name: 'Inviter' })).toBeEnabled()
  },
}

const promoting = fakeMembers()
const confirmPromoting = fakeConfirm(true)
export const PromotingAMember: Story = {
  decorators: [withMembers(promoting, confirmPromoting)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByText('bob')).toBeInTheDocument())
    await userEvent.click(memberRow(canvas, 'bob').getByRole('button', { name: 'Promouvoir' }))

    await waitFor(() =>
      expect(confirmPromoting.ask).toHaveBeenCalledWith(
        expect.objectContaining({
          message: "Voulez-vous promouvoir bob en administrateur de l'organisation ?",
        }),
      ),
    )
    await waitFor(() =>
      expect(promoting.setOrganizationAdmin).toHaveBeenCalledWith('org-acme', 'u-bob', true),
    )
    expect(toast.success).toHaveBeenCalledWith(
      "bob est désormais administrateur·rice de l'organisation.",
    )
    expect(promoting.list).toHaveBeenCalledTimes(2)
  },
}

const demoting = fakeMembers()
const confirmDemoting = fakeConfirm(true)
export const DemotingAnAdmin: Story = {
  decorators: [withMembers(demoting, confirmDemoting)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByText('alice')).toBeInTheDocument())
    await userEvent.click(memberRow(canvas, 'alice').getByRole('button', { name: 'Rétrograder' }))

    await waitFor(() =>
      expect(confirmDemoting.ask).toHaveBeenCalledWith(
        expect.objectContaining({
          message: "Voulez-vous rétrograder alice de son rôle d'administrateur de l'organisation ?",
          danger: true,
        }),
      ),
    )
    await waitFor(() =>
      expect(demoting.setOrganizationAdmin).toHaveBeenCalledWith('org-acme', 'u-alice', false),
    )
    expect(toast.success).toHaveBeenCalledWith(
      "alice n'est plus administrateur·rice de l'organisation.",
    )
  },
}

const declined = fakeMembers()
const confirmDeclined = fakeConfirm(false)
export const RoleChangeDeclined: Story = {
  decorators: [withMembers(declined, confirmDeclined)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByText('bob')).toBeInTheDocument())
    await userEvent.click(memberRow(canvas, 'bob').getByRole('button', { name: 'Promouvoir' }))

    await waitFor(() => expect(confirmDeclined.ask).toHaveBeenCalled())
    expect(declined.setOrganizationAdmin).not.toHaveBeenCalled()
    expect(toast.success).not.toHaveBeenCalled()
  },
}

export const RoleChangeFailed: Story = {
  decorators: [
    withMembers(
      fakeMembers({ setOrganizationAdmin: fn(() => throwError(() => new Error('boom'))) }),
    ),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByText('bob')).toBeInTheDocument())
    await userEvent.click(memberRow(canvas, 'bob').getByRole('button', { name: 'Promouvoir' }))

    await waitFor(() =>
      expect(toast.error).toHaveBeenCalledWith(
        "Échec de la mise à jour du statut d'administrateur.",
      ),
    )
    expect(toast.success).not.toHaveBeenCalled()
  },
}
