import { displayName } from './user.entity'

describe('displayName', () => {
  const user = {
    username: 'invite-0123456789ab',
    email: 'alice@example.com',
    invitation_pending: true,
  }

  it('is the address while the invitation is pending', () => {
    expect(displayName(user)).toBe('alice@example.com')
  })

  it('is the username once the account is activated', () => {
    expect(displayName({ ...user, username: 'alice', invitation_pending: false })).toBe('alice')
  })

  it('falls back to the username when a pending account has no address', () => {
    expect(displayName({ ...user, email: null })).toBe('invite-0123456789ab')
  })
})
