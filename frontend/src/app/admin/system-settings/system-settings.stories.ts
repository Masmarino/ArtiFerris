import { moduleMetadata, type Meta, type StoryObj } from '@storybook/angular-vite'
import { expect, fn, userEvent, waitFor, within } from 'storybook/test'
import { signal } from '@angular/core'
import { NEVER, of, throwError } from 'rxjs'
import { SystemSettingsAdmin } from './system-settings'
import { SystemSettingsService } from '../application/system-settings.service'
import { ToastService } from '../../shared/toast.service'
import { MeService } from '../../shell/application/me.service'
import { PUBLIC_ORGANIZATION_ID } from '../domain/organization.entity'
import type { SystemSettings } from '../domain/system-settings.entity'

const SETTINGS: SystemSettings = {
  max_login_attempts: 5,
  login_attempt_window_seconds: 900,
  session_ttl_hours: 24,
  registration_enabled: true,
  seo_indexing_enabled: false,
  seo_indexing_blocked: false,
  public_page_enabled: true,
}

function fakeSettings(
  overrides: Partial<SystemSettingsService> = {},
): Partial<SystemSettingsService> {
  return {
    get: fn(() => of(SETTINGS)),
    update: fn(() => of(undefined)),
    ...overrides,
  }
}

const toast = { success: fn(), error: fn() }

function withSettings(settings: Partial<SystemSettingsService>) {
  return moduleMetadata({ providers: [{ provide: SystemSettingsService, useValue: settings }] })
}

function asUser(isSuperAdmin: boolean) {
  return moduleMetadata({
    providers: [
      {
        provide: MeService,
        useValue: { isSuperAdmin: signal(isSuperAdmin), organizationId: signal(null) },
      },
    ],
  })
}

const ATTEMPTS = 'Tentatives de connexion max'
const WINDOW = 'Fenêtre de blocage (secondes)'
const SESSION = 'Durée de session (heures)'

async function waitForForm(canvas: ReturnType<typeof within>) {
  await waitFor(() => expect(canvas.getByLabelText(ATTEMPTS)).toBeInTheDocument())
}

async function replaceValue(input: HTMLElement, value: string) {
  await userEvent.clear(input)
  await userEvent.type(input, value)
}

const meta: Meta<SystemSettingsAdmin> = {
  title: 'Admin/SystemSettingsAdmin',
  component: SystemSettingsAdmin,
  beforeEach: () => {
    toast.success.mockClear()
    toast.error.mockClear()
  },
  decorators: [
    moduleMetadata({
      providers: [
        { provide: SystemSettingsService, useValue: fakeSettings() },
        { provide: ToastService, useValue: toast },
        {
          provide: MeService,
          useValue: { isSuperAdmin: signal(false), organizationId: signal(null) },
        },
      ],
    }),
  ],
}
export default meta

type Story = StoryObj<SystemSettingsAdmin>

/** Current values loaded into the form, no errors shown before a save is attempted. */
export const Loaded: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitForForm(canvas)
    expect(canvas.getByLabelText(ATTEMPTS)).toHaveValue('5')
    expect(canvas.getByLabelText(WINDOW)).toHaveValue('900')
    expect(canvas.getByLabelText(SESSION)).toHaveValue('24')
    expect(canvas.getByLabelText('Autoriser la création de compte')).toBeChecked()
    expect(canvas.getByRole('button', { name: 'Enregistrer' })).toBeEnabled()
  },
}

export const RegistrationDisabled: Story = {
  decorators: [
    withSettings(fakeSettings({ get: () => of({ ...SETTINGS, registration_enabled: false }) })),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitForForm(canvas)
    expect(canvas.getByLabelText('Autoriser la création de compte')).not.toBeChecked()
  },
}

export const Loading: Story = {
  decorators: [withSettings(fakeSettings({ get: () => NEVER }))],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByText('Chargement…')).toBeInTheDocument())
    expect(canvas.queryByLabelText(ATTEMPTS)).not.toBeInTheDocument()
    expect(canvas.queryByRole('button', { name: 'Enregistrer' })).not.toBeInTheDocument()
  },
}

const scopedSettings = fakeSettings()
/** Embedded in one organization's admin page: read and write are scoped to it. */
export const ScopedToAnOrganization: Story = {
  args: { organizationId: 'org-acme' },
  decorators: [withSettings(scopedSettings)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitForForm(canvas)
    expect(scopedSettings.get).toHaveBeenCalledWith('org-acme')

    await userEvent.click(canvas.getByRole('button', { name: 'Enregistrer' }))
    await waitFor(() => expect(scopedSettings.update).toHaveBeenCalledWith(SETTINGS, 'org-acme'))
  },
}

const outOfRangeSettings = fakeSettings()
/** Every field is bounded; an out-of-range value blocks the save and names the allowed range. */
export const OutOfRangeValues: Story = {
  decorators: [withSettings(outOfRangeSettings)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitForForm(canvas)
    await replaceValue(canvas.getByLabelText(ATTEMPTS), '0')
    await replaceValue(canvas.getByLabelText(WINDOW), '86401')
    await replaceValue(canvas.getByLabelText(SESSION), '721')
    await userEvent.click(canvas.getByRole('button', { name: 'Enregistrer' }))

    expect(await canvas.findByText('Doit être entre 1 et 1000.')).toBeInTheDocument()
    expect(canvas.getByText('Doit être entre 1 et 86400.')).toBeInTheDocument()
    expect(canvas.getByText('Doit être entre 1 et 720.')).toBeInTheDocument()
    expect(canvas.getByRole('button', { name: 'Enregistrer' })).toBeDisabled()
    expect(outOfRangeSettings.update).not.toHaveBeenCalled()
  },
}

const notANumberSettings = fakeSettings()
export const NotAnInteger: Story = {
  decorators: [withSettings(notANumberSettings)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitForForm(canvas)
    await replaceValue(canvas.getByLabelText(ATTEMPTS), '2.5')
    await userEvent.clear(canvas.getByLabelText(SESSION))
    await userEvent.click(canvas.getByRole('button', { name: 'Enregistrer' }))

    const errors = await canvas.findAllByText('Doit être un nombre entier.')
    expect(errors).toHaveLength(2)
    expect(notANumberSettings.update).not.toHaveBeenCalled()
  },
}

const savingSettings = fakeSettings({ update: fn(() => NEVER) })
export const Saving: Story = {
  decorators: [withSettings(savingSettings)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitForForm(canvas)
    await userEvent.click(canvas.getByRole('button', { name: 'Enregistrer' }))

    const button = await canvas.findByRole('button', { name: /Enregistrer/ })
    await waitFor(() => expect(button).toHaveAttribute('aria-busy', 'true'))
    expect(button).toBeDisabled()
  },
}

const savedSettings = fakeSettings()
export const SavedSuccessfully: Story = {
  decorators: [withSettings(savedSettings)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitForForm(canvas)
    await replaceValue(canvas.getByLabelText(ATTEMPTS), '10')
    await replaceValue(canvas.getByLabelText(SESSION), '8')
    await userEvent.click(canvas.getByLabelText('Autoriser la création de compte'))
    await userEvent.click(canvas.getByRole('button', { name: 'Enregistrer' }))

    await waitFor(() =>
      expect(savedSettings.update).toHaveBeenCalledWith(
        {
          max_login_attempts: 10,
          login_attempt_window_seconds: 900,
          session_ttl_hours: 8,
          registration_enabled: false,
          seo_indexing_enabled: false,
          seo_indexing_blocked: false,
          public_page_enabled: true,
        },
        undefined,
      ),
    )
    expect(toast.success).toHaveBeenCalledWith('Paramètres enregistrés.')
    expect(canvas.getByRole('button', { name: 'Enregistrer' })).toBeEnabled()
  },
}

export const SaveFailed: Story = {
  decorators: [
    withSettings(fakeSettings({ update: fn(() => throwError(() => new Error('boom'))) })),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitForForm(canvas)
    await replaceValue(canvas.getByLabelText(ATTEMPTS), '10')
    await userEvent.click(canvas.getByRole('button', { name: 'Enregistrer' }))

    await waitFor(() =>
      expect(toast.error).toHaveBeenCalledWith('Échec de la mise à jour des paramètres.'),
    )
    expect(toast.success).not.toHaveBeenCalled()
    expect(canvas.getByLabelText(ATTEMPTS)).toHaveValue('10')
    expect(canvas.getByRole('button', { name: 'Enregistrer' })).toBeEnabled()
  },
}

const SEO = "Autoriser l'indexation par les moteurs de recherche"

const seoOffSettings = fakeSettings()
/** A super-admin on the public organization can switch search-engine indexing on. */
export const SeoIndexingOff: Story = {
  args: { organizationId: PUBLIC_ORGANIZATION_ID },
  decorators: [withSettings(seoOffSettings), asUser(true)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitForForm(canvas)
    expect(canvas.getByRole('heading', { name: 'Référencement' })).toBeInTheDocument()
    expect(canvas.getByLabelText(SEO)).not.toBeChecked()
    expect(canvas.getByText(/le catalogue public envoie « noindex »/)).toBeInTheDocument()

    await userEvent.click(canvas.getByLabelText(SEO))
    await userEvent.click(canvas.getByRole('button', { name: 'Enregistrer' }))

    await waitFor(() =>
      expect(seoOffSettings.update).toHaveBeenCalledWith(
        { ...SETTINGS, seo_indexing_enabled: true },
        PUBLIC_ORGANIZATION_ID,
      ),
    )
    expect(toast.success).toHaveBeenCalledWith('Paramètres enregistrés.')
  },
}

const seoOnSettings = fakeSettings({
  get: () => of({ ...SETTINGS, seo_indexing_enabled: true }),
})
export const SeoIndexingOn: Story = {
  args: { organizationId: PUBLIC_ORGANIZATION_ID },
  decorators: [withSettings(seoOnSettings), asUser(true)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitForForm(canvas)
    expect(canvas.getByLabelText(SEO)).toBeChecked()

    await userEvent.click(canvas.getByLabelText(SEO))
    await userEvent.click(canvas.getByRole('button', { name: 'Enregistrer' }))

    await waitFor(() =>
      expect(seoOnSettings.update).toHaveBeenCalledWith(
        { ...SETTINGS, seo_indexing_enabled: false },
        PUBLIC_ORGANIZATION_ID,
      ),
    )
  },
}

const seoHiddenForAdminSettings = fakeSettings({
  get: () => of({ ...SETTINGS, seo_indexing_enabled: true }),
})
/** Only a super-admin sees the section; the loaded value still goes back untouched on save. */
export const SeoSectionHiddenForNonSuperAdmin: Story = {
  args: { organizationId: PUBLIC_ORGANIZATION_ID },
  decorators: [withSettings(seoHiddenForAdminSettings), asUser(false)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitForForm(canvas)
    expect(canvas.queryByRole('heading', { name: 'Référencement' })).not.toBeInTheDocument()
    expect(canvas.queryByLabelText(SEO)).not.toBeInTheDocument()

    await userEvent.click(canvas.getByRole('button', { name: 'Enregistrer' }))
    await waitFor(() =>
      expect(seoHiddenForAdminSettings.update).toHaveBeenCalledWith(
        { ...SETTINGS, seo_indexing_enabled: true },
        PUBLIC_ORGANIZATION_ID,
      ),
    )
  },
}

/** The switch lives on the public organization only. */
export const SeoSectionHiddenOnAnotherOrganization: Story = {
  args: { organizationId: 'org-acme' },
  decorators: [asUser(true)],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitForForm(canvas)
    expect(canvas.queryByRole('heading', { name: 'Référencement' })).not.toBeInTheDocument()
    expect(canvas.queryByLabelText(SEO)).not.toBeInTheDocument()
  },
}
