import { HttpErrorResponse } from '@angular/common/http'
import { moduleMetadata, type Meta, type StoryObj } from '@storybook/angular-vite'
import { expect, fn, userEvent, waitFor, within } from 'storybook/test'
import { of, throwError } from 'rxjs'
import { ApiTokensList } from './api-tokens-list'
import { ApiTokensApplicationService } from '../application/api-tokens.application-service'
import { ConfirmService } from '../../shared/confirm.service'
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
    expect(canvas.getByRole('button', { name: 'Nouveau token' })).toBeEnabled()
  },
}

export const Empty: Story = {
  decorators: [withTokens(fakeTokens({ list: () => of([]) }))],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    expect(await canvas.findByText('Aucun token')).toBeInTheDocument()
    expect(canvas.getByRole('button', { name: 'Nouveau token' })).toBeEnabled()
    expect(canvas.queryByRole('table')).not.toBeInTheDocument()
  },
}

/** The "Créer" button stays disabled until the token has a name. */
export const CreateFormRequiresALabel: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('button', { name: 'Nouveau token' }))
    const create = await canvas.findByRole('button', { name: 'Créer' })
    expect(create).toBeDisabled()
    await userEvent.type(canvas.getByLabelText(/Nom \(ex\. « mon laptop »\)/), 'CI runner')
    await waitFor(() => expect(create).toBeEnabled())
  },
}

export const CancellingTheCreateForm: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('button', { name: 'Nouveau token' }))
    expect(await canvas.findByRole('heading', { name: 'Nouveau token API' })).toBeInTheDocument()
    await userEvent.click(canvas.getByRole('button', { name: 'Fermer' }))
    await waitFor(() =>
      expect(canvas.queryByRole('heading', { name: 'Nouveau token API' })).not.toBeInTheDocument(),
    )
  },
}

let listAfterCreate: ApiToken[] = [LAPTOP]
const createTokens = fakeTokens({ list: fn(() => of(listAfterCreate)) })

/** The secret is only shown once, in a dedicated modal; the list is reloaded behind it. Without a password the token lasts 7 days. */
export const CreatingAToken: Story = {
  decorators: [withTokens(createTokens)],
  beforeEach: () => {
    listAfterCreate = [LAPTOP]
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('button', { name: 'Nouveau token' }))
    await userEvent.type(await canvas.findByLabelText(/Nom \(ex\. « mon laptop »\)/), 'CI runner')
    listAfterCreate = [LAPTOP, CI]
    await userEvent.click(canvas.getByRole('button', { name: 'Créer' }))

    expect(await canvas.findByText('artiferris_pat_abc123')).toBeInTheDocument()
    expect(createTokens.create).toHaveBeenCalledWith('CI runner', null)
    expect(canvas.getByRole('heading', { name: 'Token créé' })).toBeInTheDocument()
    expect(canvas.getByText('Il expirera dans 7 jours.')).toBeInTheDocument()
    expect(canvas.getByText('CI runner')).toBeInTheDocument()
    expect(canvas.queryByRole('heading', { name: 'Nouveau token API' })).not.toBeInTheDocument()

    await userEvent.click(canvas.getByRole('button', { name: "J'ai copié le token" }))
    await waitFor(() => expect(canvas.queryByText('artiferris_pat_abc123')).not.toBeInTheDocument())
  },
}

const passwordTokens = fakeTokens()

/** Typing a password switches the hint from 7 to 365 days and sends the password with the request. */
export const CreatingALongLivedToken: Story = {
  decorators: [withTokens(passwordTokens)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('button', { name: 'Nouveau token' }))
    await userEvent.type(await canvas.findByLabelText(/Nom \(ex\. « mon laptop »\)/), 'CI runner')
    expect(canvas.getByText(/expirera dans 7 jours/)).toBeInTheDocument()
    await userEvent.type(
      canvas.getByLabelText('Mot de passe (pour un jeton de longue durée)'),
      'hunter2',
    )
    expect(await canvas.findByText('Ce token expirera dans 365 jours.')).toBeInTheDocument()
    await userEvent.click(canvas.getByRole('button', { name: 'Créer' }))

    expect(await canvas.findByText('Il expirera dans 365 jours.')).toBeInTheDocument()
    expect(passwordTokens.create).toHaveBeenCalledWith('CI runner', 'hunter2')
  },
}

const wrongPasswordTokens = fakeTokens({
  create: fn(() =>
    throwError(
      () => new HttpErrorResponse({ status: 400, error: { error: 'invalid credentials' } }),
    ),
  ),
})

export const WrongPasswordForALongLivedToken: Story = {
  decorators: [withTokens(wrongPasswordTokens)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('button', { name: 'Nouveau token' }))
    await userEvent.type(await canvas.findByLabelText(/Nom \(ex\. « mon laptop »\)/), 'CI runner')
    await userEvent.type(
      canvas.getByLabelText('Mot de passe (pour un jeton de longue durée)'),
      'nope',
    )
    await userEvent.click(canvas.getByRole('button', { name: 'Créer' }))

    expect(await canvas.findByRole('alert')).toHaveTextContent('Mot de passe incorrect.')
    expect(canvas.getByRole('heading', { name: 'Nouveau token API' })).toBeInTheDocument()
  },
}

const createFailsTokens = fakeTokens({ create: fn(() => throwError(() => new Error('500'))) })

/** The form stays open with an error and the button is usable again so the user can retry. */
export const CreateFailed: Story = {
  decorators: [withTokens(createFailsTokens)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByRole('button', { name: 'Nouveau token' }))
    await userEvent.type(await canvas.findByLabelText(/Nom \(ex\. « mon laptop »\)/), 'CI runner')
    await userEvent.click(canvas.getByRole('button', { name: 'Créer' }))

    await waitFor(() => expect(createFailsTokens.create).toHaveBeenCalledWith('CI runner', null))
    expect(await canvas.findByRole('alert')).toHaveTextContent('Échec de la création du token.')
    await waitFor(() => expect(canvas.getByRole('button', { name: 'Créer' })).toBeEnabled())
    expect(canvas.getByRole('heading', { name: 'Nouveau token API' })).toBeInTheDocument()
    expect(canvas.queryByRole('heading', { name: 'Token créé' })).not.toBeInTheDocument()
  },
}

let listAfterRevoke: ApiToken[] = [LAPTOP, CI]
const revokeTokens = fakeTokens({ list: fn(() => of(listAfterRevoke)) })
const acceptingConfirm = { ask: fn(() => Promise.resolve(true)) }

/** Clicking a row asks for confirmation, then revokes that token and reloads the list. */
export const RevokingAToken: Story = {
  decorators: [withTokens(revokeTokens, acceptingConfirm)],
  beforeEach: () => {
    listAfterRevoke = [LAPTOP, CI]
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const row = await canvas.findByText('CI runner')
    listAfterRevoke = [LAPTOP]
    await userEvent.click(row)

    await waitFor(() => expect(canvas.queryByText('CI runner')).not.toBeInTheDocument())
    expect(acceptingConfirm.ask).toHaveBeenCalledWith(
      expect.objectContaining({
        heading: 'Révoquer le token',
        message: expect.stringContaining('"CI runner"'),
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
    await userEvent.click(await canvas.findByText('CI runner'))
    await waitFor(() => expect(decliningConfirm.ask).toHaveBeenCalled())
    expect(decliningTokens.revoke).not.toHaveBeenCalled()
    expect(canvas.getByText('CI runner')).toBeInTheDocument()
    expect(canvas.queryByRole('alert')).not.toBeInTheDocument()
  },
}

export const RevokeFailed: Story = {
  decorators: [
    withTokens(fakeTokens({ revoke: () => throwError(() => new Error('500')) }), {
      ask: fn(() => Promise.resolve(true)),
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(await canvas.findByText('CI runner'))
    expect(await canvas.findByRole('alert')).toHaveTextContent(
      'Échec de la révocation du token "CI runner".',
    )
    expect(canvas.getByText('CI runner')).toBeInTheDocument()
  },
}
