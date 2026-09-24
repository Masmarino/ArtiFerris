import { Provider } from '@angular/core'
import { REPOSITORY_PORT } from '../application/repository.port'
import { HttpRepositoryAdapter } from './http-repository.adapter'
import { PERMISSION_PORT } from '../application/permission.port'
import { HttpPermissionAdapter } from './http-permission.adapter'
import { PERSONAL_REPOSITORY_PORT } from '../application/personal-repository.port'
import { HttpPersonalRepositoryAdapter } from './http-personal-repository.adapter'

export const repositoryProviders: Provider[] = [
  { provide: REPOSITORY_PORT, useClass: HttpRepositoryAdapter },
  { provide: PERMISSION_PORT, useClass: HttpPermissionAdapter },
  { provide: PERSONAL_REPOSITORY_PORT, useClass: HttpPersonalRepositoryAdapter },
]
