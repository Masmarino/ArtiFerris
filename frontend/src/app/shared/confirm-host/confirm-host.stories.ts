import { Component, inject, input, signal, type OnInit } from '@angular/core'
import { type Meta, type StoryObj } from '@storybook/angular-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { ConfirmService, type ConfirmOptions } from '../confirm.service'
import { ConfirmHost } from './confirm-host'

@Component({
  selector: 'app-confirm-demo',
  standalone: true,
  imports: [ConfirmHost],
  template: `<app-confirm-host />
    <p>Réponse : {{ answer() }}</p>`,
})
class ConfirmDemo implements OnInit {
  readonly options = input.required<ConfirmOptions>()
  protected readonly answer = signal('en attente')
  private readonly confirm = inject(ConfirmService)

  ngOnInit(): void {
    void this.confirm.ask(this.options()).then((ok) => this.answer.set(ok ? 'oui' : 'non'))
  }
}

const meta: Meta<ConfirmDemo> = {
  title: 'Shared/ConfirmHost',
  component: ConfirmDemo,
}
export default meta

type Story = StoryObj<ConfirmDemo>

export const Standard: Story = {
  args: {
    options: { heading: 'Retirer du groupe', message: 'Retirer « acme-npm » du groupe ?' },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByText('Retirer « acme-npm » du groupe ?')).toBeInTheDocument()
    expect(canvas.getByRole('button', { name: 'Annuler' })).toBeInTheDocument()
    expect(canvas.getByRole('button', { name: 'Confirmer' })).toBeInTheDocument()
  },
}

export const Destructive: Story = {
  args: {
    options: {
      heading: 'Supprimer la version',
      message: 'Supprimer la version 1.2.0 de left-pad ?',
      confirmLabel: 'Supprimer',
      danger: true,
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByRole('button', { name: 'Supprimer' })).toBeInTheDocument()
  },
}

export const Confirming: Story = {
  args: {
    options: {
      heading: 'Révoquer le jeton',
      message: 'Révoquer le jeton « CI » ?',
      confirmLabel: 'Révoquer',
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('button', { name: 'Révoquer' }))
    await waitFor(() => expect(canvas.getByText('Réponse : oui')).toBeInTheDocument())
    expect(canvas.queryByRole('button', { name: 'Révoquer' })).not.toBeInTheDocument()
  },
}

export const Cancelling: Story = {
  args: {
    options: { heading: 'Révoquer le jeton', message: 'Révoquer le jeton « CI » ?' },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('button', { name: 'Annuler' }))
    await waitFor(() => expect(canvas.getByText('Réponse : non')).toBeInTheDocument())
  },
}

export const TypeToConfirm: Story = {
  args: {
    options: {
      heading: 'Supprimer le dépôt',
      message:
        'Cette action est irréversible. Le dépôt « acme-web » et son contenu seront supprimés.',
      confirmLabel: 'Supprimer',
      typeToConfirm: 'acme-web',
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const confirmButton = await canvas.findByRole('button', { name: 'Supprimer' })
    expect(confirmButton).toBeDisabled()
    await userEvent.type(canvas.getByLabelText('Saisissez « acme-web » pour confirmer'), 'acme-we')
    expect(confirmButton).toBeDisabled()
    await userEvent.type(canvas.getByLabelText('Saisissez « acme-web » pour confirmer'), 'b')
    await waitFor(() => expect(confirmButton).toBeEnabled())
    await userEvent.click(confirmButton)
    await waitFor(() => expect(canvas.getByText('Réponse : oui')).toBeInTheDocument())
  },
}

export const TypeToConfirmCancelled: Story = {
  args: {
    options: {
      heading: 'Supprimer l’utilisateur',
      message: 'Supprimer l’utilisateur « alice » ?',
      confirmLabel: 'Supprimer',
      typeToConfirm: 'alice',
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('button', { name: 'Annuler' }))
    await waitFor(() => expect(canvas.getByText('Réponse : non')).toBeInTheDocument())
  },
}
