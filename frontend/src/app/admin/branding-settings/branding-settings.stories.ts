import { moduleMetadata, type Meta, type StoryObj } from '@storybook/angular-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { of, throwError } from 'rxjs'
import { BrandingSettingsAdmin } from './branding-settings'
import { BrandingService } from '../application/branding.service'

function fakeBranding(overrides: Partial<BrandingService> = {}): Partial<BrandingService> {
  return {
    getLogo: () => of(new Blob(['logo-bytes'], { type: 'image/png' })),
    getFavicon: () => of(new Blob(['favicon-bytes'], { type: 'image/png' })),
    uploadLogo: () => of(undefined),
    uploadFavicon: () => of(undefined),
    resetLogo: () => of(undefined),
    resetFavicon: () => of(undefined),
    ...overrides,
  }
}

function pngFile(name: string): File {
  return new File(['fake-image-bytes'], name, { type: 'image/png' })
}

const meta: Meta<BrandingSettingsAdmin> = {
  title: 'Admin/BrandingSettingsAdmin',
  component: BrandingSettingsAdmin,
  decorators: [
    moduleMetadata({ providers: [{ provide: BrandingService, useValue: fakeBranding() }] }),
  ],
}
export default meta

type Story = StoryObj<BrandingSettingsAdmin>

/** Renders the current logo and favicon previews fetched from the authenticated endpoint. */
export const Default: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByAltText('Logo actuel')).toBeInTheDocument())
    expect(canvas.getByAltText('Favicon actuel')).toBeInTheDocument()
  },
}

/** Picking a logo file (via gbt-file-upload) enables the import button, which uploads it and refreshes the preview. */
export const UploadingALogo: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByAltText('Logo actuel')).toBeInTheDocument())

    const [logoInput] = canvasElement.querySelectorAll<HTMLInputElement>('.gbt-file-upload__input')
    await userEvent.upload(logoInput, pngFile('logo.png'))

    const importButton = canvas.getAllByRole('button', { name: 'Importer' })[0]
    await waitFor(() => expect(importButton).toBeEnabled())
    await userEvent.click(importButton)

    // Uploading clears the picked file, so the button disables again.
    await waitFor(() => expect(importButton).toBeDisabled())
  },
}

/** A rejected upload reports the error as a toast (asserted in this component's own unit spec,
 * which can reach ToastService directly — a play function can't). What's observable here is the
 * DOM consequence: unlike a successful upload, the picked file is kept selected so the user can
 * retry, instead of being cleared. */
export const UploadFailed: Story = {
  decorators: [
    moduleMetadata({
      providers: [
        {
          provide: BrandingService,
          useValue: fakeBranding({
            uploadLogo: () => throwError(() => ({ error: { error: 'format non supporté' } })),
          }),
        },
      ],
    }),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByAltText('Logo actuel')).toBeInTheDocument())

    const [logoInput] = canvasElement.querySelectorAll<HTMLInputElement>('.gbt-file-upload__input')
    await userEvent.upload(logoInput, pngFile('logo.png'))
    const importButton = canvas.getAllByRole('button', { name: 'Importer' })[0]
    await userEvent.click(importButton)

    await waitFor(() => expect(importButton).toBeEnabled())
    expect(canvas.getByText('logo.png')).toBeInTheDocument()
  },
}

/** Resetting reverts the logo to ArtiFerris's own default. */
export const ResettingTheLogo: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await waitFor(() => expect(canvas.getByAltText('Logo actuel')).toBeInTheDocument())
    await userEvent.click(canvas.getAllByRole('button', { name: 'Réinitialiser' })[0])
  },
}
