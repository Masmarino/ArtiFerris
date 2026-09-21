import { moduleMetadata, type Meta, type StoryObj } from '@storybook/angular-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { vi } from 'vitest'
import { of, throwError } from 'rxjs'
import { ExportAdmin } from './export'
import { ExportService } from '../application/export.service'
import type { ImportReport } from '../domain/export.entity'

const REPORT: ImportReport = {
  users_created: 2,
  repositories_created: 1,
  permissions_granted: 3,
  invited: ['admin'],
  skipped_no_email: ['member'],
  failed: [],
}

function fakeExport(overrides: Partial<ExportService> = {}): Partial<ExportService> {
  return {
    exportConfiguration: () => of(new Blob(['{}'], { type: 'application/json' })),
    importConfiguration: () => of(REPORT),
    ...overrides,
  }
}

function configFile(): File {
  return new File(['{}'], 'config.json', { type: 'application/json' })
}

const meta: Meta<ExportAdmin> = {
  title: 'Admin/ExportAdmin',
  component: ExportAdmin,
  decorators: [moduleMetadata({ providers: [{ provide: ExportService, useValue: fakeExport() }] })],
}
export default meta

type Story = StoryObj<ExportAdmin>

/** Downloading the configuration export triggers a browser download, no error shown. */
export const DownloadingTheExport: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    vi.spyOn(URL, 'createObjectURL').mockReturnValue('blob:mock')
    vi.spyOn(URL, 'revokeObjectURL').mockImplementation(() => undefined)
    await userEvent.click(
      canvas.getByRole('button', { name: 'Télécharger la configuration (JSON)' }),
    )
    await waitFor(() => expect(URL.createObjectURL).toHaveBeenCalled())
    expect(canvas.queryByRole('alert')).not.toBeInTheDocument()
  },
}

export const ExportFailed: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: ExportService,
          useValue: fakeExport({ exportConfiguration: () => throwError(() => new Error('down')) }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(
      canvas.getByRole('button', { name: 'Télécharger la configuration (JSON)' }),
    )
    await waitFor(() =>
      expect(canvas.getByRole('alert')).toHaveTextContent("Échec de l'export de la configuration."),
    )
  },
}

/** Confirming the import shows the resulting report. */
export const ImportingAConfiguration: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    vi.spyOn(window, 'confirm').mockReturnValue(true)
    const fileInput = canvasElement.querySelector<HTMLInputElement>('.gbt-file-upload__input')!
    await userEvent.upload(fileInput, configFile())
    await userEvent.click(canvas.getByRole('button', { name: 'Importer' }))
    await waitFor(() => expect(canvas.getByText(/admin/)).toBeInTheDocument())
    expect(canvas.getByText(/2 utilisateur/)).toBeInTheDocument()
    expect(canvas.getByText(/member/)).toBeInTheDocument()
  },
}

/** Declining the confirmation dialog makes no request and shows no report. */
export const DecliningTheConfirmation: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    vi.spyOn(window, 'confirm').mockReturnValue(false)
    const fileInput = canvasElement.querySelector<HTMLInputElement>('.gbt-file-upload__input')!
    await userEvent.upload(fileInput, configFile())
    await userEvent.click(canvas.getByRole('button', { name: 'Importer' }))
    expect(canvas.queryByText(/permission\(s\) restaurés/)).not.toBeInTheDocument()
  },
}

export const ImportFailed: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: ExportService,
          useValue: fakeExport({
            importConfiguration: () =>
              throwError(() => ({ error: { error: 'instance non vide' } })),
          }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    vi.spyOn(window, 'confirm').mockReturnValue(true)
    const fileInput = canvasElement.querySelector<HTMLInputElement>('.gbt-file-upload__input')!
    await userEvent.upload(fileInput, configFile())
    await userEvent.click(canvas.getByRole('button', { name: 'Importer' }))
    await waitFor(() => expect(canvas.getByRole('alert')).toHaveTextContent('instance non vide'))
  },
}
