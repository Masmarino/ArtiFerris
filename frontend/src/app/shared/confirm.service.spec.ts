import { TestBed } from '@angular/core/testing'
import { Router, provideRouter } from '@angular/router'
import { SessionToken } from '../auth/application/session-token'
import { ConfirmService } from './confirm.service'

describe('ConfirmService', () => {
  beforeEach(() => {
    sessionStorage.clear()
    TestBed.configureTestingModule({ providers: [provideRouter([{ path: '**', children: [] }])] })
  })

  function service() {
    return TestBed.inject(ConfirmService)
  }

  it('starts with nothing pending', () => {
    expect(service().pending()).toBeNull()
  })

  it('exposes the request while waiting for an answer', () => {
    const confirm = service()
    void confirm.ask({ heading: 'Supprimer', message: 'Sûr ?' })

    expect(confirm.pending()?.options).toEqual({ heading: 'Supprimer', message: 'Sûr ?' })
  })

  it('resolves true when the user confirms, and clears the request', async () => {
    const confirm = service()
    const answer = confirm.ask({ heading: 'Supprimer', message: 'Sûr ?' })

    confirm.answer(true)

    expect(await answer).toBe(true)
    expect(confirm.pending()).toBeNull()
  })

  it('resolves false when the user cancels', async () => {
    const confirm = service()
    const answer = confirm.ask({ heading: 'Supprimer', message: 'Sûr ?' })

    confirm.answer(false)

    expect(await answer).toBe(false)
  })

  it('cancels the previous request when a new one arrives', async () => {
    const confirm = service()
    const first = confirm.ask({ heading: 'Premier', message: 'a' })
    const second = confirm.ask({ heading: 'Second', message: 'b' })

    expect(await first).toBe(false)
    expect(confirm.pending()?.options.heading).toBe('Second')

    confirm.answer(true)
    expect(await second).toBe(true)
  })

  it('ignores an answer when nothing is pending', () => {
    const confirm = service()

    expect(() => confirm.answer(true)).not.toThrow()
    expect(confirm.pending()).toBeNull()
  })

  it('answers false when the user navigates away, e.g. with the Back button', async () => {
    const confirm = service()
    const answer = confirm.ask({ heading: 'Supprimer', message: 'Sûr ?' })

    await TestBed.inject(Router).navigateByUrl('/elsewhere')

    expect(await answer).toBe(false)
    expect(confirm.pending()).toBeNull()
  })

  it('answers false when the session ends', async () => {
    const confirm = service()
    const session = TestBed.inject(SessionToken)
    session.set('alice-token')
    TestBed.tick()
    const answer = confirm.ask({ heading: 'Supprimer', message: 'Sûr ?' })

    session.clear()
    TestBed.tick()

    expect(await answer).toBe(false)
    expect(confirm.pending()).toBeNull()
  })

  it('answers false when a different user signs in', async () => {
    const confirm = service()
    const session = TestBed.inject(SessionToken)
    session.set('alice-token')
    TestBed.tick()
    const answer = confirm.ask({ heading: 'Supprimer', message: 'Sûr ?' })

    session.set('bob-token')
    TestBed.tick()

    expect(await answer).toBe(false)
  })

  it('leaves a request alone when nothing changed', () => {
    const confirm = service()
    void confirm.ask({ heading: 'Supprimer', message: 'Sûr ?' })

    TestBed.tick()

    expect(confirm.pending()).not.toBeNull()
  })
})
