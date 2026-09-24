import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  computed,
  inject,
  input,
  signal,
} from '@angular/core'
import { Button } from '@masmarino/gabarit'

const COPIED_FEEDBACK_MS = 2000

@Component({
  selector: 'app-copyable-command',
  standalone: true,
  imports: [Button],
  templateUrl: './copyable-command.html',
  styleUrl: './copyable-command.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class CopyableCommand {
  private readonly destroyRef = inject(DestroyRef)
  private copiedTimer: ReturnType<typeof setTimeout> | null = null

  readonly command = input.required<string>()
  /** Accessible name of the copy button. */
  readonly label = input.required<string>()

  readonly copyState = signal<'idle' | 'copied' | 'failed'>('idle')
  readonly copyAnnouncement = computed(() => {
    switch (this.copyState()) {
      case 'copied':
        return 'Commande copiée'
      case 'failed':
        return 'Copie impossible, sélectionnez la commande manuellement'
      default:
        return ''
    }
  })

  constructor() {
    this.destroyRef.onDestroy(() => this.clearTimer())
  }

  async copy(): Promise<void> {
    try {
      await navigator.clipboard.writeText(this.command())
      this.copyState.set('copied')
    } catch {
      this.copyState.set('failed')
    }
    this.clearTimer()
    this.copiedTimer = setTimeout(() => this.copyState.set('idle'), COPIED_FEEDBACK_MS)
  }

  private clearTimer(): void {
    if (this.copiedTimer !== null) {
      clearTimeout(this.copiedTimer)
      this.copiedTimer = null
    }
  }
}
