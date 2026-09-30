export type RepositoryFormat = 'npm' | 'docker'
export type RepositoryType = 'hosted' | 'proxy' | 'group'
export type RepositoryRole = 'read' | 'write' | 'admin'

export interface RepositorySummary {
  id: string
  name: string
  format: RepositoryFormat
  repo_type: RepositoryType
  remote_url: string | null
  remote_credentials_set: boolean
  group_members: string[]
  /** `null` means unlimited; absent from the public view. */
  quota_bytes?: number | null
  /** `null` disables cleanup; absent from the public view. */
  retention_keep_last_n?: number | null
  is_public: boolean
  my_role: RepositoryRole | null
  organization_id?: string
  owner_name: string
  owner_is_personal: boolean
  /**
   * Path of the public page (e.g. `/@alice/libs`), `null` while private. Optional so fixtures stay
   * valid.
   */
  public_path?: string | null
}

export interface CreateRepositoryOptions {
  remoteUsername?: string | null
  remotePassword?: string | null
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
  /** The package has more versions than the list carries (capped at 200). */
  truncated: boolean
  vulnerability_summary: VulnerabilitySummary
}

export interface DockerImageTreeEntry {
  image_name: string
  tags: string[]
  /** The image has more tags than the list carries (capped at 100). */
  truncated: boolean
  vulnerability_summary: VulnerabilitySummary
}

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
  versions: NpmVersionDetail[]
  truncated: boolean
  dist_tags: NpmDistTagDetail[]
  readme_html: string | null
  registry_url: string
  downloads_7d: number
}

export interface DockerTagDetail {
  tag: string
  digest: string
  media_type: string
  created_at: string
  size_bytes: number | null
}

export interface DockerImageDetails {
  image_name: string
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
