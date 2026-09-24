import { componentWrapperDecorator, type Meta, type StoryObj } from '@storybook/angular-vite'
import { expect, spyOn, userEvent, waitFor, within } from 'storybook/test'
import { CopyableCommand } from './copyable-command'

const meta: Meta<CopyableCommand> = {
  title: 'Shared/CopyableCommand',
  component: CopyableCommand,
  args: {
    command: 'npm install @acme/ui --registry http://localhost:4200/npm/u/alice/ui-kit/',
    label: "Copier la commande d'installation de @acme/ui",
  },
}
export default meta

type Story = StoryObj<CopyableCommand>

export const Default: Story = {
  play: async ({ canvasElement }) => {
    expect(canvasElement.querySelector('pre')).toHaveTextContent(
      'npm install @acme/ui --registry http://localhost:4200/npm/u/alice/ui-kit/',
    )
    expect(within(canvasElement).getByRole('button', { name: /Copier/ })).toBeInTheDocument()
  },
}

/** A command longer than the container scrolls instead of pushing the button out. */
export const LongCommand: Story = {
  args: {
    command: `docker pull localhost:4200/o/a-long-organization-name/a-long-repository-name/team/service/api:${'v1.0.0-rc.1-'.repeat(8)}`,
  },
  decorators: [
    componentWrapperDecorator((story) => `<div style="max-width: 360px">${story}</div>`),
  ],
}

/** Copy puts the exact command on the clipboard and confirms it. */
export const Copied: Story = {
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    const writeText = spyOn(navigator.clipboard, 'writeText').mockResolvedValue(undefined)

    await userEvent.click(canvas.getByRole('button', { name: args.label }))

    await waitFor(() => expect(writeText).toHaveBeenCalledWith(args.command))
    expect(await canvas.findByText('Commande copiée')).toBeInTheDocument()
    writeText.mockRestore()
  },
}

/** Without clipboard access the failure is reported instead of swallowed. */
export const CopyFailed: Story = {
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement)
    const writeText = spyOn(navigator.clipboard, 'writeText').mockRejectedValue(new Error('denied'))

    await userEvent.click(canvas.getByRole('button', { name: args.label }))

    expect(await canvas.findByText(/Copie impossible/)).toBeInTheDocument()
    writeText.mockRestore()
  },
}
