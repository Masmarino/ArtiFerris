import { TestBed } from '@angular/core/testing'
import { ConfirmService } from '../../shared/confirm.service'
import { PermissionRoleEditor } from './permission-role-editor'

async function render(
  overrides: {
    confirmAnswer?: boolean
    isOpen?: boolean
    label?: string
    currentRole?: 'read' | 'write' | 'admin'
    subjectKind?: 'user' | 'repository'
  } = {},
) {
  const ask = vi.fn().mockResolvedValue(overrides.confirmAnswer ?? true)
  TestBed.configureTestingModule({ providers: [{ provide: ConfirmService, useValue: { ask } }] })
  const fixture = TestBed.createComponent(PermissionRoleEditor)
  fixture.componentRef.setInput('isOpen', overrides.isOpen ?? true)
  fixture.componentRef.setInput('label', overrides.label ?? 'florian')
  fixture.componentRef.setInput('currentRole', overrides.currentRole ?? 'read')
  if (overrides.subjectKind !== undefined) {
    fixture.componentRef.setInput('subjectKind', overrides.subjectKind)
  }
  fixture.detectChanges()
  // NgModel syncs its initial value one microtask after the first change-detection pass.
  await fixture.whenStable()
  fixture.detectChanges()
  return Object.assign(fixture, { ask })
}

describe('PermissionRoleEditor', () => {
  it('pre-selects the current role', async () => {
    const fixture = await render({ currentRole: 'write' })
    const trigger: HTMLButtonElement = fixture.nativeElement.querySelector('[role="combobox"]')
    expect(trigger.textContent).toContain('write')
  })

  it('emits roleChanged exactly once with the newly selected role when saved', async () => {
    const fixture = await render({ currentRole: 'read' })
    const roleChanged = vi.fn()
    fixture.componentInstance.roleChanged.subscribe(roleChanged)

    const trigger: HTMLButtonElement = fixture.nativeElement.querySelector('[role="combobox"]')
    trigger.click()
    fixture.detectChanges()
    const options: HTMLButtonElement[] = Array.from(
      fixture.nativeElement.querySelectorAll('[role="option"]'),
    )
    options.find((o) => o.textContent?.includes('admin'))!.click()
    fixture.detectChanges()
    const saveButton: HTMLButtonElement = fixture.nativeElement.querySelector(
      '[data-testid="save-role"] button',
    )
    saveButton.click()

    expect(roleChanged).toHaveBeenCalledTimes(1)
    expect(roleChanged).toHaveBeenCalledWith('admin')
  })

  it('emits revoked exactly once when the revoke button is clicked and confirmed', async () => {
    const fixture = await render({ label: 'florian' })
    const revoked = vi.fn()
    fixture.componentInstance.revoked.subscribe(revoked)

    const revokeButton: HTMLButtonElement = fixture.nativeElement.querySelector(
      '[data-testid="revoke"] button',
    )
    revokeButton.click()
    await fixture.whenStable()

    expect(fixture.ask).toHaveBeenCalledTimes(1)
    expect(fixture.ask).toHaveBeenCalledWith(
      expect.objectContaining({
        heading: "Révoquer l'accès",
        message: 'Révoquer l\'accès de "florian" ?',
        danger: true,
      }),
    )
    expect(revoked).toHaveBeenCalledTimes(1)
  })

  it('does not emit revoked when the confirmation is cancelled', async () => {
    const fixture = await render({ confirmAnswer: false })
    const revoked = vi.fn()
    fixture.componentInstance.revoked.subscribe(revoked)

    const revokeButton: HTMLButtonElement = fixture.nativeElement.querySelector(
      '[data-testid="revoke"] button',
    )
    revokeButton.click()
    await fixture.whenStable()

    expect(fixture.ask).toHaveBeenCalledTimes(1)
    expect(revoked).not.toHaveBeenCalled()
  })

  it('phrases the title and revoke confirmation for a user subject by default', async () => {
    const fixture = await render({ label: 'florian' })

    expect(fixture.nativeElement.textContent).toContain("Modifier l'accès de florian")
  })

  it('phrases the title and revoke confirmation for a repository subject', async () => {
    const fixture = await render({ label: 'my-repo', subjectKind: 'repository' })

    expect(fixture.nativeElement.textContent).toContain("Modifier l'accès du dépôt my-repo")

    const revokeButton: HTMLButtonElement = fixture.nativeElement.querySelector(
      '[data-testid="revoke"] button',
    )
    revokeButton.click()
    await fixture.whenStable()

    expect(fixture.ask).toHaveBeenCalledWith(
      expect.objectContaining({ message: 'Révoquer l\'accès du dépôt "my-repo" ?' }),
    )
  })

  it('emits closed when the modal reports closed', async () => {
    const fixture = await render()
    const closed = vi.fn()
    fixture.componentInstance.closed.subscribe(closed)

    fixture.nativeElement.querySelector('button[aria-label="Fermer"]').click()

    expect(closed).toHaveBeenCalled()
  })
})
