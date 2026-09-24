import { type Meta, type StoryObj } from '@storybook/angular-vite'
import { expect, userEvent, within } from 'storybook/test'
import { ConfirmModal } from './confirm-modal'

const meta: Meta<ConfirmModal> = {
  title: 'Shared/ConfirmModal',
  component: ConfirmModal,
}
export default meta

type Story = StoryObj<ConfirmModal>

export const Default: Story = {
  args: {
    heading: 'Confirm Action',
    message: 'Are you sure you want to proceed with this action?',
  },
}

export const DeleteConfirmation: Story = {
  args: {
    heading: 'Delete Item?',
    message: 'This action cannot be undone. The item will be permanently deleted.',
    confirmLabel: 'Delete',
  },
}

export const CustomLabel: Story = {
  args: {
    heading: 'Create Repository',
    message: 'Create your new personal repository?',
    confirmLabel: 'Créer son dépôt',
  },
}

export const Loading: Story = {
  args: {
    heading: 'Confirm Action',
    message: 'Processing your request...',
    confirming: true,
  },
}

export const ConfirmationFlow: Story = {
  args: {
    heading: 'Confirm Deletion',
    message: 'This item will be permanently deleted. Continue?',
    confirmLabel: 'Delete',
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const button = await canvas.findByRole('button', { name: 'Delete' })
    expect(button).toBeInTheDocument()
    await userEvent.click(button)
  },
}
