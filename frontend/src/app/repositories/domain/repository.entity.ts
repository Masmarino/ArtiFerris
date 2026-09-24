export type RepositoryFormat = 'npm' | 'docker'
export type RepositoryType = 'hosted' | 'proxy' | 'group'
export type RepositoryRole = 'read' | 'write' | 'admin'

export interface RepositorySummary {
  id: string
  name: string
  format: RepositoryFormat
  repo_type: RepositoryType
  remote_url: string | null
  /** Whether a remote username/password is configured — never the credentials themselves. */
  remote_credentials_set: boolean
  group_members: string[]
  /** `null` means unlimited. Absent from the public view (anonymous or implicit public read). */
  quota_bytes?: number | null
  /** `null` means automatic cleanup is disabled. Absent from the public view. */
  retention_keep_last_n?: number | null
  is_public: boolean
  /** The current user's own role on this repository. `null` only for an anonymous caller on a public repository. */
  my_role: RepositoryRole | null
  /** Absent from the public view. */
  organization_id?: string
  owner_name: string
  owner_is_personal: boolean
  /** The path of this repository's public page (e.g. `/@alice/libs`), `null` while the repository is private. Always present on a real response; optional here only so existing fixtures do not all need updating. */
  public_path?: string | null
}

export interface CreateRepositoryOptions {
  remoteUsername?: string | null
  remotePassword?: string | null
  /** For a `group` repository: member repository ids, in resolution order. */
  groupMembers?: string[]
  quotaBytes?: number | null
  retentionKeepLastN?: number | null
}

export interface NpmPackageVersionEntry {
  version: string
  published_at: string
  size_bytes: number
  deprecated: boolean
}

export interface VulnerabilitySummary {
  critical: number
  high: number
  medium: number
  low: number
}

export interface NpmPackageTreeEntry {
  name: string
  versions: NpmPackageVersionEntry[]
  /** The package has more versions than the list carries (the server caps it at 200). */
  truncated: boolean
  vulnerability_summary: VulnerabilitySummary
}

export interface DockerImageTreeEntry {
  image_name: string
  tags: string[]
  /** The image has more tags than the list carries (the server caps it at 100). */
  truncated: boolean
  vulnerability_summary: VulnerabilitySummary
}

/** `next_after` is the cursor for the next page, `null` or absent on the last one. */
export type RepositoryPackages =
  | { format: 'npm'; packages: NpmPackageTreeEntry[]; next_after?: string | null }
  | { format: 'docker'; images: DockerImageTreeEntry[]; next_after?: string | null }

export interface NpmVersionDetail {
  version: string
  published_at: string
  size_bytes: number
  deprecated: boolean
  deprecated_message: string | null
  shasum: string
}

export interface NpmDistTagDetail {
  tag: string
  version: string
}

export interface NpmPackageDetails {
  name: string
  /** Newest first, capped at 200: see `truncated`. */
  versions: NpmVersionDetail[]
  truncated: boolean
  dist_tags: NpmDistTagDetail[]
  /** Sanitized by the backend, `null` when the package has no README. */
  readme_html: string | null
  /** The owner's registry URL, with a trailing slash. */
  registry_url: string
  /** Downloads over the last 7 days, indicative only. */
  downloads_7d: number
}

export interface DockerTagDetail {
  tag: string
  digest: string
  media_type: string
  created_at: string
  /** `null` for a multi-arch index or when unknown. */
  size_bytes: number | null
}

export interface DockerImageDetails {
  image_name: string
  /** Without tag or scheme. */
  image_reference: string
  /** The 100 most recently updated tags: see `truncated`. */
  tags: DockerTagDetail[]
  truncated: boolean
  downloads_7d: number
}

export interface NpmAdvisory {
  id: number
  url: string
  title: string
  severity: string
  vulnerable_versions: string
  cwe: string[]
  cvss_score: number | null
}

export interface NpmDependencyAuditFinding {
  dependency_name: string
  dependency_version: string
  advisory: NpmAdvisory
}

export interface NpmDependencyAuditResult {
  scanned_at: string
  packages_scanned: number
  truncated: boolean
  findings: NpmDependencyAuditFinding[]
}

export interface DockerVulnerability {
  id: string
  package_name: string
  installed_version: string
  fixed_version: string | null
  severity: string
  title: string | null
  primary_url: string | null
}

export interface DockerImageScanResult {
  scanned_at: string
  vulnerabilities: DockerVulnerability[]
}
