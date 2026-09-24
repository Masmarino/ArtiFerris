import { dockerPullCommand, npmInstallCommand, preferredTag } from './install-commands'

describe('npmInstallCommand', () => {
  it('points npm at the registry URL verbatim', () => {
    expect(npmInstallCommand('@alice/button', 'http://localhost:4200/npm/u/alice/ui-kit/')).toBe(
      'npm install @alice/button --registry http://localhost:4200/npm/u/alice/ui-kit/',
    )
  })
})

describe('npmInstallCommand with hostile names', () => {
  it('puts an option-shaped name after -- and quotes it', () => {
    expect(npmInstallCommand('--global', 'http://localhost:4200/npm/u/alice/ui-kit/')).toBe(
      "npm install --registry http://localhost:4200/npm/u/alice/ui-kit/ -- '--global'",
    )
  })

  it('quotes a name with spaces or shell metacharacters', () => {
    expect(npmInstallCommand('a b;$(id)', 'http://r/')).toBe(
      "npm install 'a b;$(id)' --registry http://r/",
    )
  })

  it('quotes a registry URL that is not a plain word', () => {
    expect(npmInstallCommand('button', 'http://r/?a=1&b=2')).toBe(
      "npm install button --registry 'http://r/?a=1&b=2'",
    )
  })
})

describe('dockerPullCommand', () => {
  it('appends the tag', () => {
    expect(dockerPullCommand('localhost:4200/u/alice/repo/hello', 'v1')).toBe(
      'docker pull localhost:4200/u/alice/repo/hello:v1',
    )
  })

  it('puts an option-shaped reference after -- and quotes it', () => {
    expect(dockerPullCommand('--privileged', 'v1')).toBe("docker pull -- '--privileged:v1'")
  })

  it('quotes a reference with shell metacharacters', () => {
    expect(dockerPullCommand('host/a b', null)).toBe("docker pull 'host/a b'")
  })

  it('leaves the reference alone without a tag', () => {
    expect(dockerPullCommand('localhost:4200/u/alice/repo/hello', null)).toBe(
      'docker pull localhost:4200/u/alice/repo/hello',
    )
  })
})

describe('preferredTag', () => {
  it('prefers latest wherever it sits in the list', () => {
    expect(preferredTag([{ tag: 'v2' }, { tag: 'latest' }, { tag: 'v1' }])).toBe('latest')
  })

  it('falls back to the first tag', () => {
    expect(preferredTag([{ tag: 'v2' }, { tag: 'v1' }])).toBe('v2')
  })

  it('is null without tags', () => {
    expect(preferredTag([])).toBeNull()
  })
})
