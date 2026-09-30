import { t } from '../../shared/i18n/translator'
import { TranslocoPipe } from '@jsverse/transloco'
import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  effect,
  inject,
  input,
  signal,
  untracked,
  type WritableSignal,
} from '@angular/core'
import { FormsModule } from '@angular/forms'
import { Observable } from 'rxjs'
import { Button, Card, FileUpload, Tooltip } from '@masmarino/gabarit'
import { BrandingService } from '../application/branding.service'
import { ToastService } from '../../shared/toast.service'
import { rejectionMessage } from '../../shared/api-error'

// Kept in sync with MAX_ASSET_BYTES in branding.rs — the maxSizeMb below rejects an oversized file client-side to save the full upload round-trip just to be told no.
const MAX_ASSET_MB = 2

@Component({
  selector: 'app-branding-settings',
  standalone: true,
  imports: [TranslocoPipe, Button, Card, FileUpload, FormsModule, Tooltip],
  templateUrl: './branding-settings.html',
  styleUrl: './branding-settings.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class BrandingSettingsAdmin {
  private readonly brandingService = inject(BrandingService)
  private readonly destroyRef = inject(DestroyRef)
  private readonly toastService = inject(ToastService)

  readonly organizationId = input<string | undefined>(undefined)

  // Object URLs from the authenticated preview endpoint, not the public host-resolved one.
  readonly logoPreviewUrl = signal<string | null>(null)
  readonly faviconPreviewUrl = signal<string | null>(null)

  readonly selectedLogoFile = signal<File[]>([])
  readonly uploadingLogo = signal(false)
  readonly logoError = signal<string | null>(null)

  readonly selectedFaviconFile = signal<File[]>([])
  readonly uploadingFavicon = signal(false)
  readonly faviconError = signal<string | null>(null)

  readonly maxSizeMb = MAX_ASSET_MB
  readonly removeFileLabel = (name: string): string => t('admin.branding.removeFile', { name })
  readonly oversizeMessage = (name: string, maxSizeMb: number): string =>
    t('admin.branding.oversize', { name, max: maxSizeMb })

  // effect(), not ngOnInit — this component is reused across organizations on the same route.
  constructor() {
    effect(() => {
      this.organizationId()
      untracked(() => {
        this.clearPreview(this.logoPreviewUrl)
        this.clearPreview(this.faviconPreviewUrl)
        this.selectedLogoFile.set([])
        this.selectedFaviconFile.set([])
        this.uploadingLogo.set(false)
        this.uploadingFavicon.set(false)
        this.reloadLogo()
        this.reloadFavicon()
      })
    })
    this.destroyRef.onDestroy(() => {
      this.revokePreview(this.logoPreviewUrl())
      this.revokePreview(this.faviconPreviewUrl())
    })
  }

  private reloadLogo(): void {
    this.loadPreview((id) => this.brandingService.getLogo(id), this.logoPreviewUrl)
  }

  private reloadFavicon(): void {
    this.loadPreview((id) => this.brandingService.getFavicon(id), this.faviconPreviewUrl)
  }

  private loadPreview(
    fetch: (organizationId?: string) => Observable<Blob>,
    target: WritableSignal<string | null>,
  ): void {
    const organizationId = this.organizationId()
    fetch(organizationId).subscribe({
      next: (blob) => {
        if (this.organizationId() === organizationId) {
          this.setPreview(target, blob)
        }
      },
      // Without a preview the upload and reset buttons still work.
      error: () => undefined,
    })
  }

  private clearPreview(target: WritableSignal<string | null>): void {
    this.revokePreview(target())
    target.set(null)
  }

  private setPreview(target: WritableSignal<string | null>, blob: Blob): void {
    // untracked: reading target() here is bookkeeping to revoke the old object URL, not a
    // dependency this should establish — reading it un-tracked avoids the effect above
    // re-triggering itself every time this same signal it just wrote to changes.
    const previous = untracked(target)
    target.set(URL.createObjectURL(blob))
    this.revokePreview(previous)
  }

  private revokePreview(url: string | null): void {
    if (url) {
      URL.revokeObjectURL(url)
    }
  }

  uploadLogo(): void {
    const file = this.selectedLogoFile()[0]
    if (!file || this.uploadingLogo()) return
    this.uploadingLogo.set(true)
    this.logoError.set(null)
    this.brandingService.uploadLogo(file, this.organizationId()).subscribe({
      next: () => {
        this.uploadingLogo.set(false)
        this.selectedLogoFile.set([])
        this.reloadLogo()
        this.toastService.success(t('admin.branding.logoImported'))
      },
      error: (err) => {
        this.uploadingLogo.set(false)
        this.toastService.error(rejectionMessage(err) ?? t('admin.branding.errors.logoImport'))
      },
    })
  }

  resetLogo(): void {
    if (this.uploadingLogo()) return
    this.uploadingLogo.set(true)
    this.logoError.set(null)
    this.brandingService.resetLogo(this.organizationId()).subscribe({
      next: () => {
        this.uploadingLogo.set(false)
        this.reloadLogo()
        this.toastService.success(t('admin.branding.logoReset'))
      },
      error: () => {
        this.uploadingLogo.set(false)
        this.toastService.error(t('admin.branding.errors.logoReset'))
      },
    })
  }

  uploadFavicon(): void {
    const file = this.selectedFaviconFile()[0]
    if (!file || this.uploadingFavicon()) return
    this.uploadingFavicon.set(true)
    this.faviconError.set(null)
    this.brandingService.uploadFavicon(file, this.organizationId()).subscribe({
      next: () => {
        this.uploadingFavicon.set(false)
        this.selectedFaviconFile.set([])
        this.reloadFavicon()
        this.toastService.success(t('admin.branding.faviconImported'))
      },
      error: (err) => {
        this.uploadingFavicon.set(false)
        this.toastService.error(rejectionMessage(err) ?? t('admin.branding.errors.faviconImport'))
      },
    })
  }

  resetFavicon(): void {
    if (this.uploadingFavicon()) return
    this.uploadingFavicon.set(true)
    this.faviconError.set(null)
    this.brandingService.resetFavicon(this.organizationId()).subscribe({
      next: () => {
        this.uploadingFavicon.set(false)
        this.reloadFavicon()
        this.toastService.success(t('admin.branding.faviconReset'))
      },
      error: () => {
        this.uploadingFavicon.set(false)
        this.toastService.error(t('admin.branding.errors.faviconReset'))
      },
    })
  }
}
