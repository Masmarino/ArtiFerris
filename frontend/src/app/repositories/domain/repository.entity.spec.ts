import { RepositorySummary } from './repository.entity'

describe('RepositorySummary', () => {
  it('accepts is_public and a null my_role, matching the backend response shape', () => {
    const anonymousView: RepositorySummary = {
      id: 'repo-1',
      name: 'my-lib',
      format: 'npm',
      repo_type: 'hosted',
      remote_url: null,
      remote_credentials_set: false,
      group_members: [],
      quota_bytes: null,
      retention_keep_last_n: null,
      is_public: true,
      my_role: null,
      organization_id: 'org-1',
      owner_name: 'alice',
      owner_is_personal: true,
    }

    expect(anonymousView.is_public).toBe(true)
    expect(anonymousView.my_role).toBeNull()
  })
})
