import { InjectionToken } from '@angular/core'
import { Observable } from 'rxjs'
import { MeResponse } from '../domain/me.entity'

export interface MePort {
  load(): Observable<MeResponse>
  /** The change ends every session, the caller's included: the answer is a fresh one for the caller. */
  changePassword(currentPassword: string, newPassword: string): Observable<{ token: string }>
  setLanguage(language: string): Observable<void>
}

export const ME_PORT = new InjectionToken<MePort>('MePort')
