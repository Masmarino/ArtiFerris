import { ChangeDetectionStrategy, Component, inject, signal } from '@angular/core'
import { FormsModule } from '@angular/forms'
import { Button, Card, FileUpload } from '@masmarino/gabarit'
import { ExportService } from '../application/export.service'
import { ImportReport } from '../domain/export.entity'
import { downloadBlob } from '../../shared/download'

@Component({
  selector: 'app-export',
  standalone: true,
  imports: [Button, Card, FileUpload, FormsModule],
  templateUrl: './export.html',
  styleUrl: './export.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ExportAdmin {
  private readonly exportService = inject(ExportService)

  readonly downloading = signal(false)
  readonly error = signal<string | null>(null)

  readonly selectedFile = signal<File[]>([])
  readonly importing = signal(false)
  readonly importError = signal<string | null>(null)
  readonly importReport = signal<ImportReport | null>(null)
  readonly removeFileLabel = (name: string): string => `Retirer ${name}`

  downloadConfiguration(): void {
    this.error.set(null)
    this.downloading.set(true)
    this.exportService.exportConfiguration().subscribe({
      next: (blob) => {
        this.downloading.set(false)
        downloadBlob(blob, `artiferris-config-${new Date().toISOString().slice(0, 10)}.json`)
      },
      error: () => {
        this.downloading.set(false)
        this.error.set("Échec de l'export de la configuration.")
      },
    })
  }

  onFileSelected(files: File[]): void {
    this.selectedFile.set(files)
    this.importReport.set(null)
    this.importError.set(null)
  }

  importConfiguration(): void {
    const file = this.selectedFile()[0]
    if (!file || this.importing()) return
    if (
      !confirm(
        'Importer cette configuration ? Cette opération ne fonctionne que sur une instance vide (sans dépôt, sans autre utilisateur que le vôtre).',
      )
    ) {
      return
    }
    this.importing.set(true)
    this.importError.set(null)
    this.importReport.set(null)
    this.exportService.importConfiguration(file).subscribe({
      next: (report) => {
        this.importing.set(false)
        this.importReport.set(report)
      },
      error: (err) => {
        this.importing.set(false)
        this.importError.set(err?.error?.error ?? "Échec de l'import de la configuration.")
      },
    })
  }
}
