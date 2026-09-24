import { dockerPullCommand, npmInstallCommand } from '../../../repositories/domain/install-commands'
import { CatalogEntry } from './catalog.entity'

/** Built from the API's `registry_url` / `image_reference`, which already point at the owner's URL. */
export function installCommand(entry: CatalogEntry): string {
  if (entry.kind === 'npm') {
    return entry.registry_url ? npmInstallCommand(entry.name, entry.registry_url) : ''
  }
  return entry.image_reference ? dockerPullCommand(entry.image_reference, entry.latest) : ''
}
