use async_trait::async_trait;
use chrono::{DateTime, Utc};
use semver::Version;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::DomainError;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NpmPackageName(String);

impl NpmPackageName {
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        if raw.is_empty() || raw.len() > 214 {
            return Err(DomainError::Validation("package name must be 1-214 characters".into()));
        }
        let unscoped = if let Some(rest) = raw.strip_prefix('@') {
            let mut parts = rest.splitn(2, '/');
            let scope = parts.next().unwrap_or_default();
            let name = parts.next().ok_or_else(|| {
                DomainError::Validation("scoped package name must be @scope/name".into())
            })?;
            if scope.is_empty() || name.is_empty() {
                return Err(DomainError::Validation("scoped package name must be @scope/name".into()));
            }
            Self::validate_segment(scope)?;
            name
        } else {
            raw
        };
        Self::validate_segment(unscoped)?;
        Ok(Self(raw.to_string()))
    }

    fn validate_segment(segment: &str) -> Result<(), DomainError> {
        // "." and ".." pass every char check below, so reject them explicitly too.
        let valid = !segment.is_empty()
            && segment != "."
            && segment != ".."
            && segment
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_' || c == '.');
        if valid {
            Ok(())
        } else {
            Err(DomainError::Validation(format!(
                "invalid package name segment: {segment}"
            )))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The part after `@scope/`, or the whole name if unscoped — e.g. `foo` for both `foo` and `@bar/foo`.
    pub fn local_name(&self) -> &str {
        self.0.rsplit('/').next().unwrap_or(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct NpmVersion(#[serde(with = "version_serde")] Version);

impl NpmVersion {
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        Version::parse(raw)
            .map(Self)
            .map_err(|e| DomainError::Validation(format!("invalid semver version: {e}")))
    }

    pub fn as_str(&self) -> String {
        self.0.to_string()
    }

    /// The version without its build metadata, which takes no part in precedence: `1.0.0+a` and `1.0.0+b` are one release.
    pub fn release(&self) -> String {
        let mut version = self.0.clone();
        version.build = semver::BuildMetadata::EMPTY;
        version.to_string()
    }

    /// A version with a `-` suffix such as `1.0.0-beta.1`. Build metadata (`1.0.0+build-5`) does not make one.
    pub fn is_prerelease(&self) -> bool {
        !self.0.pre.is_empty()
    }
}

/// Most versions one package may hold. Every packument request loads them all.
pub const MAX_VERSIONS_PER_PACKAGE: i64 = 5000;

/// Most manifest bytes one package may hold across its versions, which bounds the size of its packument.
pub const MAX_MANIFEST_BYTES_PER_PACKAGE: i64 = 32 * 1024 * 1024;

/// Longest dist-tag name accepted.
const MAX_DIST_TAG_LENGTH: usize = 64;

/// A short word such as `latest` or `v2-lts`. A tag that reads as a version (`1.2.3`) is refused: `pkg@1.2.3` would be ambiguous.
pub fn validate_dist_tag(tag: &str) -> Result<(), DomainError> {
    let well_formed = !tag.is_empty()
        && tag.len() <= MAX_DIST_TAG_LENGTH
        && tag.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && tag.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if !well_formed {
        return Err(DomainError::Validation(format!("invalid dist-tag name: {tag:?}")));
    }
    if Version::parse(tag).is_ok() {
        return Err(DomainError::Validation(format!("a dist-tag cannot look like a version: {tag:?}")));
    }
    Ok(())
}

mod version_serde {
    use semver::Version;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &Version, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&v.to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Version, D::Error> {
        let raw = String::deserialize(d)?;
        Version::parse(&raw).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NpmPackageOrigin {
    Local,
    ProxyCache,
}

#[derive(Debug, Clone)]
pub struct NpmPackage {
    pub id: Uuid,
    pub package_repository_id: Uuid,
    pub name: NpmPackageName,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub metadata_fetched_at: Option<DateTime<Utc>>,
    pub cached_metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct NpmPackageVersion {
    pub id: Uuid,
    pub npm_package_id: Uuid,
    pub version: NpmVersion,
    pub manifest: serde_json::Value,
    pub shasum: String,
    pub integrity: String,
    pub tarball_storage_key: String,
    pub tarball_size_bytes: i64,
    pub deprecated: bool,
    pub deprecated_message: Option<String>,
    pub published_by: Option<Uuid>,
    pub published_at: DateTime<Utc>,
    pub origin: NpmPackageOrigin,
}

/// Lighter than [`NpmPackageVersion`] — no `manifest`, a full package.json a browse/retention sweep never reads.
#[derive(Debug, Clone)]
pub struct NpmPackageVersionSummary {
    pub id: Uuid,
    pub npm_package_id: Uuid,
    pub version: NpmVersion,
    pub shasum: String,
    pub tarball_size_bytes: i64,
    pub deprecated: bool,
    pub deprecated_message: Option<String>,
    pub published_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NpmDistTag {
    pub npm_package_id: Uuid,
    pub tag: String,
    pub version: NpmVersion,
}

/// What `unpublish_version` removed.
#[derive(Debug, Clone)]
pub struct UnpublishedVersion {
    pub package_id: Uuid,
    pub tarball_storage_key: String,
    /// The version was the package's last, so the package went with it.
    pub package_deleted: bool,
}

#[derive(Debug, Clone)]
pub enum UnpublishVersionOutcome {
    PackageNotFound,
    VersionNotFound,
    Removed(UnpublishedVersion),
}

/// What `unpublish_package` removed.
#[derive(Debug, Clone)]
pub struct UnpublishedPackage {
    pub package_id: Uuid,
    pub tarball_storage_keys: Vec<String>,
}

#[async_trait]
pub trait NpmPackageRepositoryPort: Send + Sync {
    async fn find_package(
        &self,
        repository_id: Uuid,
        name: &NpmPackageName,
    ) -> Result<Option<NpmPackage>, DomainError>;

    async fn find_by_id(&self, id: Uuid) -> Result<Option<NpmPackage>, DomainError>;

    /// Returns the id of the row, which is the existing one when another request created the same (repository, name) first.
    async fn create_package(&self, package: &NpmPackage) -> Result<Uuid, DomainError>;

    async fn touch_metadata_fetched_at(
        &self,
        npm_package_id: Uuid,
        fetched_at: DateTime<Utc>,
    ) -> Result<(), DomainError>;

    async fn set_cached_metadata(&self, npm_package_id: Uuid, metadata: serde_json::Value) -> Result<(), DomainError>;

    async fn list_versions(&self, npm_package_id: Uuid) -> Result<Vec<NpmPackageVersion>, DomainError>;
    /// Batched form of `list_versions` across several packages in one query.
    async fn list_versions_for_packages(&self, npm_package_ids: &[Uuid]) -> Result<Vec<NpmPackageVersionSummary>, DomainError>;
    /// Like `list_versions_for_packages`, but at most the `per_package` newest versions of each package, newest first: the cut happens
    /// in the query, so a package with thousands of versions costs no more than one with `per_package`.
    async fn list_latest_versions_for_packages(&self, npm_package_ids: &[Uuid], per_package: i64) -> Result<Vec<NpmPackageVersionSummary>, DomainError>;

    async fn find_version(
        &self,
        npm_package_id: Uuid,
        version: &NpmVersion,
    ) -> Result<Option<NpmPackageVersion>, DomainError>;

    async fn insert_version(&self, version: &NpmPackageVersion) -> Result<(), DomainError>;

    /// A publish, in one transaction under a lock on the package row (created here if it doesn't exist yet): inserts the version
    /// and points each of `dist_tags` at it. `version.npm_package_id` is replaced by the row's id, which is returned. Fails with
    /// `NpmVersionAlreadyExists` if the version (build metadata aside) exists or was unpublished before, with `NpmPackageLimit` if
    /// the package is at `MAX_VERSIONS_PER_PACKAGE` or `MAX_MANIFEST_BYTES_PER_PACKAGE`, and with `CommitFailed` if the outcome of
    /// the commit is unknown; any other error means nothing was written.
    async fn publish_version(&self, package: &NpmPackage, version: &NpmPackageVersion, dist_tags: &[String]) -> Result<Uuid, DomainError>;

    /// An unpublish, in one transaction under the same package lock as `publish_version`: remembers the version as unpublished,
    /// deletes it and the dist-tags pointing at it, and deletes the package if that was its last version.
    async fn unpublish_version(&self, repository_id: Uuid, name: &NpmPackageName, version: &NpmVersion) -> Result<UnpublishVersionOutcome, DomainError>;

    /// Like `unpublish_version` for every version of the package at once. `None` if there is no such package.
    async fn unpublish_package(&self, repository_id: Uuid, name: &NpmPackageName) -> Result<Option<UnpublishedPackage>, DomainError>;

    /// Build metadata aside: `1.0.0+b` counts as unpublished if `1.0.0` was.
    async fn was_unpublished(&self, repository_id: Uuid, name: &NpmPackageName, version: &NpmVersion) -> Result<bool, DomainError>;

    async fn set_deprecated(
        &self,
        npm_package_id: Uuid,
        version: &NpmVersion,
        message: Option<&str>,
    ) -> Result<(), DomainError>;

    async fn list_dist_tags(&self, npm_package_id: Uuid) -> Result<Vec<NpmDistTag>, DomainError>;
    /// Batched form of `list_dist_tags` across several packages in one query.
    async fn list_dist_tags_for_packages(&self, npm_package_ids: &[Uuid]) -> Result<Vec<NpmDistTag>, DomainError>;

    /// Fails with `NpmVersionNotFound` if the version is gone, checked under the package lock, so a tag never lands on a version a concurrent unpublish removed.
    async fn set_dist_tag(&self, npm_package_id: Uuid, tag: &str, version: &NpmVersion) -> Result<(), DomainError>;

    async fn delete_dist_tag(&self, npm_package_id: Uuid, tag: &str) -> Result<(), DomainError>;

    async fn search(
        &self,
        repository_id: Uuid,
        query: &str,
        limit: i64,
    ) -> Result<Vec<NpmPackage>, DomainError>;

    /// Up to `limit` packages in name order, starting after `after`.
    async fn list_packages_page(&self, repository_id: Uuid, after: Option<&str>, limit: i64) -> Result<Vec<NpmPackage>, DomainError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_simple_package_name() {
        assert!(NpmPackageName::parse("left-pad").is_ok());
    }

    #[test]
    fn accepts_a_scoped_package_name() {
        let name = NpmPackageName::parse("@artiferris/cli").unwrap();
        assert_eq!(name.as_str(), "@artiferris/cli");
    }

    #[test]
    fn rejects_an_empty_name() {
        assert!(NpmPackageName::parse("").is_err());
    }

    #[test]
    fn rejects_a_name_with_uppercase_letters() {
        assert!(NpmPackageName::parse("Left-Pad").is_err());
    }

    #[test]
    fn rejects_a_scope_without_a_slash() {
        assert!(NpmPackageName::parse("@artiferris").is_err());
    }

    #[test]
    fn rejects_a_dot_or_dot_dot_segment() {
        assert!(NpmPackageName::parse(".").is_err());
        assert!(NpmPackageName::parse("..").is_err());
        assert!(NpmPackageName::parse("@../..").is_err());
    }

    #[test]
    fn parses_a_valid_semver_version() {
        assert!(NpmVersion::parse("1.2.3").is_ok());
        assert!(NpmVersion::parse("1.2.3-beta.1").is_ok());
    }

    #[test]
    fn build_metadata_is_not_a_prerelease() {
        assert!(!NpmVersion::parse("1.0.0+build-5").unwrap().is_prerelease());
        assert!(!NpmVersion::parse("1.0.0").unwrap().is_prerelease());
        assert!(NpmVersion::parse("1.0.0-beta.1").unwrap().is_prerelease());
        assert!(NpmVersion::parse("1.0.0-rc.1+build-5").unwrap().is_prerelease());
    }

    #[test]
    fn dist_tag_names_are_short_words_that_do_not_look_like_versions() {
        for good in ["latest", "next", "v2-lts", "beta.1", "canary_2", "A1"] {
            assert!(validate_dist_tag(good).is_ok(), "{good}");
        }
        for bad in ["", "1.2.3", "1.0.0-beta.1", "-x", ".x", "a b", "a/b", "a@b", "a:b", "tag\n", &"x".repeat(65)] {
            assert!(validate_dist_tag(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn rejects_an_invalid_version() {
        assert!(NpmVersion::parse("not-a-version").is_err());
    }

    #[test]
    fn orders_versions_by_semver_not_string() {
        let v9 = NpmVersion::parse("9.0.0").unwrap();
        let v10 = NpmVersion::parse("10.0.0").unwrap();
        assert!(v9 < v10, "9.0.0 must sort before 10.0.0 under semver, not string order");
    }
}
