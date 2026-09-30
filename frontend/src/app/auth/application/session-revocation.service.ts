import { Injectable, inject } from '@angular/core'
import { Router } from '@angular/router'
import { AuthService } from './auth.service'
import { SESSIONS_ENDED_QUERY_PARAM, SESSIONS_ENDED_REASON } from '../domain/sessions-ended'

@Injectable({ providedIn: 'root' })
export class SessionRevocationService {
  private readonly auth = inject(AuthService)
  private readonly router = inject(Router)

  signOutAndRedirect(): void {
    this.auth.logout()
    void this.router.navigate(['/login'], {
      queryParams: { [SESSIONS_ENDED_QUERY_PARAM]: SESSIONS_ENDED_REASON },
    })
  }
}
