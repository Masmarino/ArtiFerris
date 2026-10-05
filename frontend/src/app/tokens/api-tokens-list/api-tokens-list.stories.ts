import { HttpErrorResponse } from '@angular/common/http'
import { moduleMetadata, type Meta, type StoryObj } from '@storybook/angular-vite'
import { expect, fn, userEvent, waitFor, within } from 'storybook/test'
import { of, throwError } from 'rxjs'
import { ApiTokensList } from './api-tokens-list'
import { ApiTokensApplicationService } from '../application/api-tokens.application-service'
import { ConfirmService } from '../../shared/confirm.service'
import { ToastService } from '../../shared/toast.service'
import type { ApiToken } from '../domain/api-token.entity'

const LAPTOP: ApiToken = {
  id: 't1',
  label: 'mon laptop',
  created_at: '2026-06-01T10:00:00Z',
  last_used_at: '2026-06-10T08:00:00Z',
}
const CI: ApiToken = {
  id: 't2',
  label: 'CI runner',
  created_at: '2026-07-15T10:00:00Z',
  last_used_at: null,
}

function fakeTokens(
  overrides: Partial<ApiTokensApplicationService> = {},
): Partial<ApiTokensApplicationService> {
  return {
    list: fn(() => of([LAPTOP, CI])),
    create: fn(() => of({ id: 't3', token: 'artiferris_pat_abc123' })),
    revoke: fn(() => of(undefined)),
    ...overrides,
  }
}

function withTokens(
  tokens: Partial<ApiTokensApplicationService>,
  confirmService?: { ask: ReturnType<typeof fn> },
) {
  return moduleMetadata({
    providers: [
      { provide: ApiTokensApplicationService, useValue: tokens },
      ...(confirmService ? [{ provide: ConfirmService, useValue: confirmService }] : []),
    ],
  })
}

const meta: Meta<ApiTokensList> = {
  title: 'Tokens/ApiTokensList',
  component: ApiTokensList,
  decorators: [withTokens(fakeTokens())],
}
export default meta

type Story = StoryObj<ApiTokensList>

export const Default: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByText('mon laptop')).toBeInTheDocument()
    expect(canvas.getByText('CI runner')).toBeInTheDocument()
    expect(canvas.getByText('Jamais utilisé')).toBeInTheDocument()
    expect(canvas.getByRole('button', { name: 'Générer' })).toBeDisabled()
  },
}

export const Empty: Story = {
  decorators: [withTokens(fakeTokens({ list: () => of([]) }))],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByText('Aucun jeton')).toBeInTheDocument()
    expect(canvas.queryByRole('list', { name: 'Jetons actifs' })).not.toBeInTheDocument()
  },
}

export const LoadFailed: Story = {
  decorators: [withTokens(fakeTokens({ list: () => throwError(() => new Error('500')) }))],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByText("Les jetons n'ont pas pu être chargés.")).toBeInTheDocument()
    expect(canvas.getByRole('button', { name: 'Réessayer' })).toBeEnabled()
  },
}

export const GeneratingNeedsAName: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const generate = await canvas.findByRole('button', { name: 'Générer' })
    expect(generate).toBeDisabled()
    await userEvent.type(canvas.getByLabelText('Nom du jeton'), 'CI runner')
    await waitFor(() => expect(generate).toBeEnabled())
  },
}

let listAfterCreate: ApiToken[] = [LAPTOP]
const createTokens = fakeTokens({ list: fn(() => of(listAfterCreate)) })

export const CreatingAToken: Story = {
  decorators: [withTokens(createTokens)],
  beforeEach: () => {
    listAfterCreate = [LAPTOP]
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.type(await canvas.findByLabelText('Nom du jeton'), 'CI runner')
    listAfterCreate = [LAPTOP, { ...CI, id: 't3' }]
    await userEvent.click(canvas.getByRole('button', { name: 'Générer' }))

    expect(await canvas.findByText('artiferris_pat_abc123')).toBeInTheDocument()
    expect(createTokens.create).toHaveBeenCalledWith('CI runner', null)
    expect(canvas.getByRole('heading', { name: 'Jeton « CI runner » généré' })).toBeInTheDocument()
    expect(canvas.getByText(/Il expirera dans 7 jours\./)).toBeInTheDocument()
    expect(canvas.getByText('Nouveau')).toBeInTheDocument()
    expect(canvas.getByLabelText('Nom du jeton')).toHaveValue('')

    await userEvent.click(canvas.getByRole('button', { name: "J'ai copié le jeton" }))
    await waitFor(() => expect(canvas.queryByText('artiferris_pat_abc123')).not.toBeInTheDocument())
  },
}

const passwordTokens = fakeTokens()

export const CreatingALongLivedToken: Story = {
  decorators: [withTokens(passwordTokens)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.type(await canvas.findByLabelText('Nom du jeton'), 'CI runner')
    expect(canvas.getByText(/expirera dans 7 jours/)).toBeInTheDocument()
    await userEvent.type(canvas.getByLabelText('Mot de passe (facultatif)'), 'hunter2')
    expect(await canvas.findByText('Ce jeton expirera dans 365 jours.')).toBeInTheDocument()
    await userEvent.click(canvas.getByRole('button', { name: 'Générer' }))

    expect(await canvas.findByText(/Il expirera dans 365 jours\./)).toBeInTheDocument()
    expect(passwordTokens.create).toHaveBeenCalledWith('CI runner', 'hunter2')
  },
}

const wrongPasswordTokens = fakeTokens({
  create: fn(() =>
    throwError(
      () =>
        new HttpErrorResponse({
          status: 400,
          error: { error: 'invalid credentials', code: 'invalid_credentials' },
        }),
    ),
  ),
})

export const WrongPasswordForALongLivedToken: Story = {
  decorators: [withTokens(wrongPasswordTokens)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.type(await canvas.findByLabelText('Nom du jeton'), 'CI runner')
    await userEvent.type(canvas.getByLabelText('Mot de passe (facultatif)'), 'nope')
    await userEvent.click(canvas.getByRole('button', { name: 'Générer' }))

    expect(await canvas.findByText('Mot de passe incorrect.')).toBeInTheDocument()
    expect(canvas.getByLabelText('Nom du jeton')).toHaveValue('CI runner')
    expect(canvas.getByLabelText('Mot de passe (facultatif)')).toHaveValue('')
  },
}

const createFailsTokens = fakeTokens({ create: fn(() => throwError(() => new Error('500'))) })

export const CreateFailed: Story = {
  decorators: [withTokens(createFailsTokens)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.type(await canvas.findByLabelText('Nom du jeton'), 'CI runner')
    await userEvent.click(canvas.getByRole('button', { name: 'Générer' }))

    await waitFor(() => expect(createFailsTokens.create).toHaveBeenCalledWith('CI runner', null))
    expect(await canvas.findByText('Impossible de générer le jeton.')).toBeInTheDocument()
    await waitFor(() => expect(canvas.getByRole('button', { name: 'Générer' })).toBeEnabled())
    expect(canvas.queryByText('artiferris_pat_abc123')).not.toBeInTheDocument()
  },
}

let listAfterRevoke: ApiToken[] = [LAPTOP, CI]
const revokeTokens = fakeTokens({ list: fn(() => of(listAfterRevoke)) })
const acceptingConfirm = { ask: fn(() => Promise.resolve(true)) }

export const RevokingAToken: Story = {
  decorators: [withTokens(revokeTokens, acceptingConfirm)],
  beforeEach: () => {
    listAfterRevoke = [LAPTOP, CI]
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const revoke = await canvas.findByRole('button', { name: 'Révoquer le jeton CI runner' })
    listAfterRevoke = [LAPTOP]
    await userEvent.click(revoke)

    await waitFor(() => expect(canvas.queryByText('CI runner')).not.toBeInTheDocument())
    expect(acceptingConfirm.ask).toHaveBeenCalledWith(
      expect.objectContaining({
        heading: 'Révoquer le jeton',
        message: expect.stringContaining('« CI runner »'),
      }),
    )
    expect(revokeTokens.revoke).toHaveBeenCalledWith('t2')
    expect(canvas.getByText('mon laptop')).toBeInTheDocument()
  },
}

const decliningTokens = fakeTokens()
const decliningConfirm = { ask: fn(() => Promise.resolve(false)) }

export const DecliningTheRevocation: Story = {
  decorators: [withTokens(decliningTokens, decliningConfirm)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(
      await canvas.findByRole('button', { name: 'Révoquer le jeton CI runner' }),
    )
    await waitFor(() => expect(decliningConfirm.ask).toHaveBeenCalled())
    expect(decliningTokens.revoke).not.toHaveBeenCalled()
    expect(canvas.getByText('CI runner')).toBeInTheDocument()
  },
}

const revokeFailedToasts = { success: fn(), error: fn() }

export const RevokeFailed: Story = {
  decorators: [
    withTokens(fakeTokens({ revoke: () => throwError(() => new Error('500')) }), {
      ask: fn(() => Promise.resolve(true)),
    }),
    moduleMetadata({ providers: [{ provide: ToastService, useValue: revokeFailedToasts }] }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(
      await canvas.findByRole('button', { name: 'Révoquer le jeton CI runner' }),
    )
    await waitFor(() =>
      expect(revokeFailedToasts.error).toHaveBeenCalledWith('Impossible de révoquer ce jeton.'),
    )
    expect(canvas.getByText('CI runner')).toBeInTheDocument()
  },
}
