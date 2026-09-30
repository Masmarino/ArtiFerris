import { moduleMetadata, type Meta, type StoryObj } from '@storybook/angular-vite'
import { expect, fn, userEvent, waitFor, within } from 'storybook/test'
import { ConfirmService } from '../../shared/confirm.service'
import { PermissionRoleEditor } from './permission-role-editor'

const meta: Meta<PermissionRoleEditor> = {
  title: 'Repositories/PermissionRoleEditor',
  component: PermissionRoleEditor,
  args: {
    isOpen: true,
    label: 'alice',
    currentRole: 'read',
    subjectKind: 'user',
    saving: false,
    roleChanged: fn(),
    revoked: fn(),
    closed: fn(),
  },
}
export default meta

type Story = StoryObj<PermissionRoleEditor>

export const ForUser: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(
      await canvas.findByRole('heading', { name: "Modifier l'accès de alice" }),
    ).toBeInTheDocument()
    expect(canvas.getByRole('combobox', { name: 'Rôle' })).toHaveTextContent('read')
  },
}

export const ForRepository: Story = {
  args: { label: 'acme-npm', subjectKind: 'repository', currentRole: 'admin' },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(
      await canvas.findByRole('heading', { name: "Modifier l'accès du dépôt acme-npm" }),
    ).toBeInTheDocument()
    expect(canvas.getByRole('combobox', { name: 'Rôle' })).toHaveTextContent('admin')
  },
}

export const Closed: Story = {
  args: { isOpen: false },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.queryByRole('heading')).not.toBeInTheDocument())
    expect(canvas.queryByRole('button', { name: 'Enregistrer' })).not.toBeInTheDocument()
  },
}

export const SavingWithTheCurrentRole: Story = {
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('button', { name: 'Enregistrer' }))
    expect(args.roleChanged).toHaveBeenCalledOnce()
    expect(args.roleChanged).toHaveBeenCalledWith('read')
  },
}

export const ChangingTheRole: Story = {
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('combobox', { name: 'Rôle' }))
    await userEvent.click(await canvas.findByRole('option', { name: 'write' }))
    await userEvent.click(canvas.getByRole('button', { name: 'Enregistrer' }))
    expect(args.roleChanged).toHaveBeenCalledOnce()
    expect(args.roleChanged).toHaveBeenCalledWith('write')
    expect(args.revoked).not.toHaveBeenCalled()
  },
}

export const Saving: Story = {
  args: { saving: true },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByRole('button', { name: /Enregistrer/ })).toBeDisabled()
    expect(canvas.getByRole('button', { name: /Révoquer/ })).toBeDisabled()
  },
}

const askAccepted = fn(() => Promise.resolve(true))
export const RevokingAfterConfirmation: Story = {
  decorators: [
    moduleMetadata({ providers: [{ provide: ConfirmService, useValue: { ask: askAccepted } }] }),
  ],
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('button', { name: 'Révoquer' }))
    await waitFor(() => expect(args.revoked).toHaveBeenCalledOnce())
    expect(askAccepted).toHaveBeenCalledWith(
      expect.objectContaining({ message: 'Révoquer l\'accès de "alice" ?', danger: true }),
    )
  },
}

const askAcceptedForRepository = fn(() => Promise.resolve(true))
export const RevokingRepositoryAccess: Story = {
  args: { label: 'acme-npm', subjectKind: 'repository' },
  decorators: [
    moduleMetadata({
      providers: [{ provide: ConfirmService, useValue: { ask: askAcceptedForRepository } }],
    }),
  ],
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('button', { name: 'Révoquer' }))
    await waitFor(() => expect(args.revoked).toHaveBeenCalledOnce())
    expect(askAcceptedForRepository).toHaveBeenCalledWith(
      expect.objectContaining({ message: 'Révoquer l\'accès du dépôt "acme-npm" ?' }),
    )
  },
}

const askDeclined = fn(() => Promise.resolve(false))
export const RevokeDeclined: Story = {
  decorators: [
    moduleMetadata({ providers: [{ provide: ConfirmService, useValue: { ask: askDeclined } }] }),
  ],
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('button', { name: 'Révoquer' }))
    await waitFor(() => expect(askDeclined).toHaveBeenCalledOnce())
    expect(args.revoked).not.toHaveBeenCalled()
  },
}

export const Closing: Story = {
  play: async ({ canvasElement, args }) => {
    await userEvent.click(await within(canvasElement).findByRole('button', { name: 'Fermer' }))
    expect(args.closed).toHaveBeenCalledOnce()
  },
}
