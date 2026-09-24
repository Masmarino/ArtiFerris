import { catalogEntry, dockerEntry } from '../testing/catalog.fixtures'
import { installCommand } from './install-command'

describe('installCommand', () => {
  it('points npm at the owner registry URL from the API, verbatim', () => {
    expect(installCommand(catalogEntry())).toBe(
      'npm install hangar-demo --registry http://localhost:4200/npm/u/admin/test-npm/',
    )
  })

  it('pulls a docker image from its reference with the latest tag', () => {
    expect(installCommand(dockerEntry())).toBe(
      'docker pull localhost:4200/o/acme/images/team/api:v2.0.1',
    )
  })

  it('omits the tag when a docker image has none', () => {
    expect(installCommand(dockerEntry({ latest: null }))).toBe(
      'docker pull localhost:4200/o/acme/images/team/api',
    )
  })

  it('never lets a package name act as an npm option', () => {
    expect(installCommand(catalogEntry({ name: '--global' }))).toBe(
      "npm install --registry http://localhost:4200/npm/u/admin/test-npm/ -- '--global'",
    )
  })
})
