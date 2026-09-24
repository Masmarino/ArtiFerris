import { describeBlockedAccount } from './blocked-account'

const ORG = '0b8f6a58-1c3e-4a7b-9d2f-5e6a7b8c9d0e'

function describe_(key: string) {
  return describeBlockedAccount({ username: key, remaining_seconds: 60 })
}

describe('describeBlockedAccount', () => {
  it('strips the shared login prefix and unlocks by the bare username', () => {
    expect(describe_('login-user:alice')).toEqual({ label: 'alice', unlockAs: 'alice' })
  })

  it('strips the organization prefix and id, and marks the row as organization-scoped', () => {
    expect(describe_(`login-org-user:${ORG}:alice`)).toEqual({
      label: 'alice (organisation)',
      unlockAs: 'alice',
    })
  })

  it('keeps colons that belong to the username itself', () => {
    expect(describe_('login-user:a:b').unlockAs).toBe('a:b')
    expect(describe_(`login-org-user:${ORG}:a:b`).unlockAs).toBe('a:b')
  })

  it.each([
    'mfa:3f2b1c00-0000-4000-8000-000000000001',
    'mfa-setup:3f2b1c00-0000-4000-8000-000000000001',
    'mfa-manage:3f2b1c00-0000-4000-8000-000000000001',
    'login-ip:203.0.113.0',
    'register:203.0.113.0',
    'bare-name',
  ])('offers no unlock for %s and shows the raw key', (key) => {
    expect(describe_(key)).toEqual({ label: key, unlockAs: null })
  })

  it.each(['login-user:', `login-org-user:${ORG}:`, 'login-org-user:short:alice'])(
    'offers no unlock for the malformed key %s',
    (key) => {
      expect(describe_(key).unlockAs).toBeNull()
    },
  )
})
