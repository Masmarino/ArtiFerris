import { TestBed } from '@angular/core/testing'
import { Router, provideRouter } from '@angular/router'
import { SessionRevocationService } from './session-revocation.service'
import { AUTH_PORT } from './auth.port'
import { SessionToken } from './session-token'

describe('SessionRevocationService', () => {
  beforeEach(() => {
    TestBed.configureTestingModule({
      providers: [provideRouter([]), { provide: AUTH_PORT, useValue: {} }],
    })
  })

  afterEach(() => {
    sessionStorage.clear()
  })

  it('drops the session token and sends the user to /login with the sessions-ended reason', () => {
    TestBed.inject(SessionToken).set('a-jwt')
    const router = TestBed.inject(Router)
    const navigate = vi.spyOn(router, 'navigate').mockResolvedValue(true)

    TestBed.inject(SessionRevocationService).signOutAndRedirect()

    expect(TestBed.inject(SessionToken).value()).toBeNull()
    expect(navigate).toHaveBeenCalledWith(['/login'], {
      queryParams: { reason: 'sessions-revoked' },
    })
  })
})
