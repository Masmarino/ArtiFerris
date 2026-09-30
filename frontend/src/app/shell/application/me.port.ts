import { InjectionToken } from '@angular/core'
import { Observable } from 'rxjs'
import { MeResponse } from '../domain/me.entity'

export interface MePort {
  load(): Observable<MeResponse>
  changePassword(currentPassword: string, newPassword: string): Observable<void>
  setLanguage(language: string): Observable<void>
}

export const ME_PORT = new InjectionToken<MePort>('MePort')
