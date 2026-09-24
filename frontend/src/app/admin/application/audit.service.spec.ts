import { TestBed } from '@angular/core/testing'
import { of } from 'rxjs'
import { AuditService } from './audit.service'
import { AUDIT_PORT, AuditPort } from './audit.port'

describe('AuditService', () => {
  function setup(port: Partial<AuditPort>) {
    TestBed.configureTestingModule({ providers: [{ provide: AUDIT_PORT, useValue: port }] })
    return TestBed.inject(AuditService)
  }

  it('delegates query() to the port, defaulting the filter to an empty object', () => {
    const query = vi.fn().mockReturnValue(of({ entries: [], next_cursor: null }))
    setup({ query }).query()

    expect(query).toHaveBeenCalledWith({})
  })

  it('delegates blockedAccounts() to the port', () => {
    const blockedAccounts = vi.fn().mockReturnValue(of([]))
    setup({ blockedAccounts }).blockedAccounts()

    expect(blockedAccounts).toHaveBeenCalled()
  })

  it('delegates unlockUsername() to the port', () => {
    const unlockUsername = vi.fn().mockReturnValue(of(undefined))
    setup({ unlockUsername }).unlockUsername('alice')

    expect(unlockUsername).toHaveBeenCalledWith('alice')
  })
})
