import { InjectionToken } from '@angular/core'
import { Observable } from 'rxjs'
import { VersionResponse } from '../domain/version.entity'

export interface VersionPort {
  load(): Observable<VersionResponse>
}

export const VERSION_PORT = new InjectionToken<VersionPort>('VersionPort')
