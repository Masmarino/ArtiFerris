use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use artiferris_domain::docker_registry::{DockerImageName, DockerManifestRepositoryPort};
use artiferris_domain::download_stats::DownloadStatsPort;
use artiferris_domain::npm_package::{NpmPackageName, NpmPackageRepositoryPort, NpmVersion};
use artiferris_domain::package_repository::RepositoryFormat;
use artiferris_domain::readme::ReadmeRendererPort;
use uuid::Uuid;

use crate::error::ApplicationError;

pub struct NpmVersionDetail {
    pub version: String,
    pub published_at: DateTime<Utc>,
    pub size_bytes: i64,
    pub deprecated: bool,
    pub deprecated_message: Option<String>,
    pub shasum: String,
}

pub struct NpmDistTagDetail {
    pub tag: String,
    pub version: String,
}

pub const MAX_VERSIONS_IN_DETAILS: usize = 200;

pub struct NpmPackageDetails {
    pub name: String,
    /// Newest first, at most `MAX_VERSIONS_IN_DETAILS`.
    pub versions: Vec<NpmVersionDetail>,
    /// The package has more versions than are listed.
    pub truncated: bool,
    pub dist_tags: Vec<NpmDistTagDetail>,
    /// The latest version's README, converted and sanitized; `None` when it has none.
    pub readme_html: Option<String>,
    pub downloads_7d: i64,
}

/// `None` when the package doesn't exist, including "existed but was fully unpublished".
pub struct GetNpmPackageDetailsUseCase {
    packages: Arc<dyn NpmPackageRepositoryPort>,
    readme_renderer: Arc<dyn ReadmeRendererPort>,
    download_stats: Arc<dyn DownloadStatsPort>,
}

impl GetNpmPackageDetailsUseCase {
    pub fn new(packages: Arc<dyn NpmPackageRepositoryPort>, readme_renderer: Arc<dyn ReadmeRendererPort>, download_stats: Arc<dyn DownloadStatsPort>) -> Self {
        Self { packages, readme_renderer, download_stats }
    }

    pub async fn execute(&self, repository_id: Uuid, name: &NpmPackageName) -> Result<Option<NpmPackageDetails>, ApplicationError> {
        let Some(package) = self.packages.find_package(repository_id, name).await? else {
            return Ok(None);
        };
        let mut versions = self.packages.list_latest_versions_for_packages(&[package.id], MAX_VERSIONS_IN_DETAILS as i64 + 1).await?;
        versions.sort_by_key(|v| std::cmp::Reverse(v.published_at));
        let dist_tags = self.packages.list_dist_tags(package.id).await?;
        // The `latest` tag wins, else the newest publication (`versions` is already newest first).
        let latest = dist_tags.iter().find(|t| t.tag == "latest").map(|t| &t.version).or_else(|| versions.first().map(|v| &v.version));
        let readme_html = match latest {
            Some(latest) => self.render_readme(package.id, latest).await?,
            None => None,
        };
        let downloads_7d = self.download_stats.downloads_last_7_days(repository_id, RepositoryFormat::Npm, name.as_str()).await?;
        let truncated = versions.len() > MAX_VERSIONS_IN_DETAILS;
        Ok(Some(NpmPackageDetails {
            name: name.as_str().to_string(),
            readme_html,
            downloads_7d,
            truncated,
            versions: versions
                .into_iter()
                .take(MAX_VERSIONS_IN_DETAILS)
                .map(|v| NpmVersionDetail {
                    version: v.version.as_str(),
                    published_at: v.published_at,
                    size_bytes: v.tarball_size_bytes,
                    deprecated: v.deprecated,
                    deprecated_message: v.deprecated_message,
                    shasum: v.shasum,
                })
                .collect(),
            dist_tags: dist_tags.into_iter().map(|t| NpmDistTagDetail { tag: t.tag, version: t.version.as_str() }).collect(),
        }))
    }

    /// Only the latest version's manifest is loaded: every version carries its own copy of the README.
    async fn render_readme(&self, package_id: Uuid, version: &NpmVersion) -> Result<Option<String>, ApplicationError> {
        let Some(latest) = self.packages.find_version(package_id, version).await? else { return Ok(None) };
        let Some(readme) = latest.manifest.get("readme").and_then(|readme| readme.as_str()).filter(|readme| !readme.trim().is_empty()) else { return Ok(None) };
        Ok(Some(self.readme_renderer.render(readme).await).filter(|html| !html.trim().is_empty()))
    }
}

pub struct DockerTagDetail {
    pub tag: String,
    pub digest: String,
    pub media_type: String,
    pub created_at: DateTime<Utc>,
    /// Config plus layers, as the manifest declares them. `None` for a manifest list or index, or an unreadable body.
    pub size_bytes: Option<i64>,
}

pub struct DockerImageDetails {
    pub image_name: String,
    pub downloads_7d: i64,
    /// The most recently updated tags, at most `MAX_TAGS_IN_DETAILS`, by tag name.
    pub tags: Vec<DockerTagDetail>,
    /// The image has more tags than are listed.
    pub truncated: bool,
}

pub const MAX_TAGS_IN_DETAILS: usize = 100;
/// A manifest body bigger than this is not read, so its tag has no size.
const MAX_SIZED_MANIFEST_BYTES: i64 = 256 * 1024;

pub struct GetDockerImageDetailsUseCase {
    manifests: Arc<dyn DockerManifestRepositoryPort>,
    download_stats: Arc<dyn DownloadStatsPort>,
}

impl GetDockerImageDetailsUseCase {
    pub fn new(manifests: Arc<dyn DockerManifestRepositoryPort>, download_stats: Arc<dyn DownloadStatsPort>) -> Self {
        Self { manifests, download_stats }
    }

    pub async fn execute(&self, repository_id: Uuid, image_name: &DockerImageName) -> Result<DockerImageDetails, ApplicationError> {
        let mut summaries = self.manifests.list_tag_manifest_summaries(repository_id, image_name, MAX_TAGS_IN_DETAILS as i64 + 1).await?;
        let truncated = summaries.len() > MAX_TAGS_IN_DETAILS;
        summaries.truncate(MAX_TAGS_IN_DETAILS);
        summaries.sort_by(|a, b| a.0.cmp(&b.0));
        let mut digests: Vec<String> = summaries.iter().map(|(_, digest, _, _)| digest.as_str().to_string()).collect();
        digests.sort();
        digests.dedup();
        let sizes: HashMap<String, Option<i64>> = self
            .manifests
            .list_tagged_manifest_bodies(repository_id, image_name, &digests, MAX_SIZED_MANIFEST_BYTES)
            .await?
            .into_iter()
            .map(|(digest, body)| (digest.as_str().to_string(), declared_size_bytes(&body)))
            .collect();
        let tags = summaries
            .into_iter()
            .map(|(tag, digest, media_type, created_at)| {
                let size_bytes = sizes.get(digest.as_str()).copied().flatten();
                DockerTagDetail { tag, digest: digest.as_str().to_string(), media_type: media_type.as_str().to_string(), created_at, size_bytes }
            })
            .collect();
        let downloads_7d = self.download_stats.downloads_last_7_days(repository_id, RepositoryFormat::Docker, image_name.as_str()).await?;
        Ok(DockerImageDetails { image_name: image_name.as_str().to_string(), downloads_7d, tags, truncated })
    }
}

/// What an image manifest says it weighs: its config blob plus every layer. A manifest list or index has no layers of its own.
fn declared_size_bytes(body: &[u8]) -> Option<i64> {
    let manifest: serde_json::Value = serde_json::from_slice(body).ok()?;
    let layers = manifest.get("layers")?.as_array()?;
    let config = manifest.get("config").and_then(|c| c.get("size")).and_then(|s| s.as_i64()).unwrap_or(0);
    layers.iter().try_fold(config, |total, layer| total.checked_add(layer.get("size")?.as_i64()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::use_cases::docker_test_support::FakeDockerManifestRepository;

    /// Reports the same weekly figure for everything.
    struct WeeklyDownloads(i64);

    #[async_trait::async_trait]
    impl DownloadStatsPort for WeeklyDownloads {
        async fn add_batch(&self, _counts: &[artiferris_domain::download_stats::DownloadCount]) -> Result<(), artiferris_domain::error::DomainError> {
            Ok(())
        }
        async fn downloads_last_7_days(&self, _repository_id: Uuid, _format: RepositoryFormat, _name: &str) -> Result<i64, artiferris_domain::error::DomainError> {
            Ok(self.0)
        }
        async fn prune_before(&self, _day: chrono::NaiveDate) -> Result<u64, artiferris_domain::error::DomainError> {
            Ok(0)
        }
    }

    /// Stands in for the real renderer, whose sanitizing is tested where it lives.
    struct UppercaseRenderer;

    #[async_trait::async_trait]
    impl ReadmeRendererPort for UppercaseRenderer {
        async fn render(&self, markdown: &str) -> String {
            format!("<p>{}</p>", markdown.to_uppercase())
        }
    }
    use crate::use_cases::npm_test_support::FakePackages;
    use chrono::Duration;
    use artiferris_domain::docker_registry::{Digest, DockerManifest, DockerMediaType};
    use artiferris_domain::npm_package::{NpmPackage, NpmPackageOrigin, NpmPackageVersion, NpmVersion};

    #[tokio::test]
    async fn returns_none_for_an_unknown_npm_package() {
        let use_case = GetNpmPackageDetailsUseCase::new(Arc::new(FakePackages::new()), Arc::new(UppercaseRenderer), Arc::new(WeeklyDownloads(42)));
        let result = use_case.execute(Uuid::new_v4(), &NpmPackageName::parse("left-pad").unwrap()).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn returns_versions_newest_first_and_dist_tags() {
        let packages = Arc::new(FakePackages::new());
        let repository_id = Uuid::new_v4();
        let name = NpmPackageName::parse("left-pad").unwrap();
        let package = NpmPackage {
            id: Uuid::new_v4(),
            package_repository_id: repository_id,
            name: name.clone(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            metadata_fetched_at: None,
            cached_metadata: None,
        };
        packages.create_package(&package).await.unwrap();
        let v1 = NpmVersion::parse("1.0.0").unwrap();
        let v2 = NpmVersion::parse("2.0.0").unwrap();
        packages
            .insert_version(&NpmPackageVersion {
                id: Uuid::new_v4(),
                npm_package_id: package.id,
                version: v1.clone(),
                manifest: serde_json::json!({}),
                shasum: "s1".to_string(),
                integrity: "i1".to_string(),
                tarball_storage_key: "k1".to_string(),
                tarball_size_bytes: 10,
                deprecated: false,
                deprecated_message: None,
                published_by: None,
                published_at: Utc::now() - Duration::days(1),
                origin: NpmPackageOrigin::Local,
            })
            .await
            .unwrap();
        packages
            .insert_version(&NpmPackageVersion {
                id: Uuid::new_v4(),
                npm_package_id: package.id,
                version: v2.clone(),
                manifest: serde_json::json!({}),
                shasum: "s2".to_string(),
                integrity: "i2".to_string(),
                tarball_storage_key: "k2".to_string(),
                tarball_size_bytes: 20,
                deprecated: true,
                deprecated_message: Some("use v3 instead".to_string()),
                published_by: None,
                published_at: Utc::now(),
                origin: NpmPackageOrigin::Local,
            })
            .await
            .unwrap();
        packages.set_dist_tag(package.id, "latest", &v2).await.unwrap();

        let use_case = GetNpmPackageDetailsUseCase::new(packages, Arc::new(UppercaseRenderer), Arc::new(WeeklyDownloads(42)));
        let details = use_case.execute(repository_id, &name).await.unwrap().unwrap();

        assert_eq!(details.name, "left-pad");
        assert_eq!(details.downloads_7d, 42);
        assert_eq!(details.versions.iter().map(|v| v.version.as_str()).collect::<Vec<_>>(), vec!["2.0.0", "1.0.0"]);
        assert!(details.versions[0].deprecated);
        assert_eq!(details.versions[0].deprecated_message.as_deref(), Some("use v3 instead"));
        assert_eq!(details.dist_tags.len(), 1);
        assert_eq!(details.dist_tags[0].tag, "latest");
        assert_eq!(details.dist_tags[0].version, "2.0.0");
    }

    #[tokio::test]
    async fn resolves_each_tag_to_its_manifests_digest_and_media_type() {
        let manifests = Arc::new(FakeDockerManifestRepository::new());
        let repository_id = Uuid::new_v4();
        let name = DockerImageName::parse("my-app").unwrap();
        let manifest = DockerManifest {
            id: Uuid::new_v4(),
            package_repository_id: repository_id,
            image_name: name.clone(),
            digest: Digest::of(b"{}"),
            media_type: DockerMediaType::OciManifest,
            body: b"{}".to_vec(),
            created_at: Utc::now(),
        };
        manifests.insert_manifest(&manifest, &[]).await.unwrap();
        manifests.set_tag(repository_id, &name, "latest", manifest.id).await.unwrap();

        let use_case = GetDockerImageDetailsUseCase::new(manifests, Arc::new(WeeklyDownloads(42)));
        let details = use_case.execute(repository_id, &name).await.unwrap();

        assert_eq!(details.image_name, "my-app");
        assert_eq!(details.downloads_7d, 42);
        assert_eq!(details.tags.len(), 1);
        assert_eq!(details.tags[0].tag, "latest");
        assert_eq!(details.tags[0].digest, manifest.digest.as_str());
        assert_eq!(details.tags[0].media_type, "application/vnd.oci.image.manifest.v1+json");
    }

    async fn package_with_versions(readmes: &[(&str, Option<serde_json::Value>, i64)], latest: Option<&str>) -> (GetNpmPackageDetailsUseCase, Uuid, NpmPackageName) {
        let packages = Arc::new(FakePackages::new());
        let repository_id = Uuid::new_v4();
        let name = NpmPackageName::parse("widget").unwrap();
        let package = NpmPackage { id: Uuid::new_v4(), package_repository_id: repository_id, name: name.clone(), created_at: Utc::now(), updated_at: Utc::now(), metadata_fetched_at: None, cached_metadata: None };
        packages.create_package(&package).await.unwrap();
        for (version, readme, days_ago) in readmes {
            let version = NpmVersion::parse(version).unwrap();
            let manifest = match readme {
                Some(readme) => serde_json::json!({ "readme": readme }),
                None => serde_json::json!({}),
            };
            packages
                .insert_version(&NpmPackageVersion {
                    id: Uuid::new_v4(),
                    npm_package_id: package.id,
                    version: version.clone(),
                    manifest,
                    shasum: "s".to_string(),
                    integrity: "i".to_string(),
                    tarball_storage_key: "k".to_string(),
                    tarball_size_bytes: 1,
                    deprecated: false,
                    deprecated_message: None,
                    published_by: None,
                    published_at: Utc::now() - Duration::days(*days_ago),
                    origin: NpmPackageOrigin::Local,
                })
                .await
                .unwrap();
            if latest == Some(version.as_str().as_str()) {
                packages.set_dist_tag(package.id, "latest", &version).await.unwrap();
            }
        }
        (GetNpmPackageDetailsUseCase::new(packages, Arc::new(UppercaseRenderer), Arc::new(WeeklyDownloads(42))), repository_id, name)
    }

    #[tokio::test]
    async fn the_readme_comes_from_the_version_the_latest_tag_points_at() {
        let (use_case, repository_id, name) = package_with_versions(&[("1.0.0", Some("stable".into()), 10), ("2.0.0-beta", Some("beta".into()), 1)], Some("1.0.0")).await;

        let details = use_case.execute(repository_id, &name).await.unwrap().unwrap();

        assert_eq!(details.readme_html.as_deref(), Some("<p>STABLE</p>"));
    }

    #[tokio::test]
    async fn without_a_latest_tag_the_newest_versions_readme_is_used() {
        let (use_case, repository_id, name) = package_with_versions(&[("1.0.0", Some("old".into()), 10), ("1.1.0", Some("new".into()), 1)], None).await;

        let details = use_case.execute(repository_id, &name).await.unwrap().unwrap();

        assert_eq!(details.readme_html.as_deref(), Some("<p>NEW</p>"));
    }

    #[tokio::test]
    async fn a_missing_blank_or_non_text_readme_means_no_readme() {
        for readme in [None, Some(serde_json::json!("")), Some(serde_json::json!("  \n ")), Some(serde_json::json!(42)), Some(serde_json::json!(["a"]))] {
            let (use_case, repository_id, name) = package_with_versions(&[("1.0.0", readme.clone(), 1)], Some("1.0.0")).await;

            let details = use_case.execute(repository_id, &name).await.unwrap().unwrap();

            assert_eq!(details.readme_html, None, "{readme:?}");
        }
    }

    #[tokio::test]
    async fn a_package_with_hundreds_of_versions_lists_the_newest_ones_and_says_so() {
        let versions: Vec<(String, Option<serde_json::Value>, i64)> = (0..MAX_VERSIONS_IN_DETAILS + 50).map(|i| (format!("1.0.{i}"), None, (MAX_VERSIONS_IN_DETAILS + 50 - i) as i64)).collect();
        let versions: Vec<(&str, Option<serde_json::Value>, i64)> = versions.iter().map(|(v, r, d)| (v.as_str(), r.clone(), *d)).collect();
        let (use_case, repository_id, name) = package_with_versions(&versions, None).await;

        let details = use_case.execute(repository_id, &name).await.unwrap().unwrap();

        assert!(details.truncated);
        assert_eq!(details.versions.len(), MAX_VERSIONS_IN_DETAILS);
        assert_eq!(details.versions[0].version, format!("1.0.{}", MAX_VERSIONS_IN_DETAILS + 49));
    }

    #[tokio::test]
    async fn the_version_cap_is_applied_by_the_query_not_after_loading_every_version() {
        let packages = Arc::new(FakePackages::new());
        let repository_id = Uuid::new_v4();
        let name = NpmPackageName::parse("widget").unwrap();
        let package = NpmPackage { id: Uuid::new_v4(), package_repository_id: repository_id, name: name.clone(), created_at: Utc::now(), updated_at: Utc::now(), metadata_fetched_at: None, cached_metadata: None };
        packages.create_package(&package).await.unwrap();
        for i in 0..300 {
            packages
                .insert_version(&NpmPackageVersion {
                    id: Uuid::new_v4(),
                    npm_package_id: package.id,
                    version: NpmVersion::parse(&format!("1.0.{i}")).unwrap(),
                    manifest: serde_json::json!({}),
                    shasum: "s".to_string(),
                    integrity: "i".to_string(),
                    tarball_storage_key: "k".to_string(),
                    tarball_size_bytes: 1,
                    deprecated: false,
                    deprecated_message: None,
                    published_by: None,
                    published_at: Utc::now() - Duration::seconds(300 - i),
                    origin: NpmPackageOrigin::Local,
                })
                .await
                .unwrap();
        }
        let use_case = GetNpmPackageDetailsUseCase::new(packages.clone(), Arc::new(UppercaseRenderer), Arc::new(WeeklyDownloads(0)));

        let details = use_case.execute(repository_id, &name).await.unwrap().unwrap();

        assert_eq!((details.versions.len(), details.truncated), (MAX_VERSIONS_IN_DETAILS, true));
        assert!(packages.capped_summaries_returned.load(std::sync::atomic::Ordering::Relaxed) <= MAX_VERSIONS_IN_DETAILS + 1);
    }

    #[tokio::test]
    async fn a_latest_tag_on_a_version_older_than_the_listed_ones_still_picks_its_readme() {
        let mut versions: Vec<(String, Option<serde_json::Value>, i64)> = (1..=MAX_VERSIONS_IN_DETAILS as i64 + 20).map(|i| (format!("1.0.{i}"), None, MAX_VERSIONS_IN_DETAILS as i64 + 21 - i)).collect();
        versions.push(("0.9.0".to_string(), Some("old stable".into()), 10_000));
        let versions: Vec<(&str, Option<serde_json::Value>, i64)> = versions.iter().map(|(v, r, d)| (v.as_str(), r.clone(), *d)).collect();
        let (use_case, repository_id, name) = package_with_versions(&versions, Some("0.9.0")).await;

        let details = use_case.execute(repository_id, &name).await.unwrap().unwrap();

        assert_eq!(details.readme_html.as_deref(), Some("<p>OLD STABLE</p>"));
    }

    #[tokio::test]
    async fn only_the_latest_versions_manifest_is_loaded_whatever_the_number_of_versions() {
        let readmes: Vec<(String, Option<serde_json::Value>, i64)> = (0..20).map(|i| (format!("1.0.{i}"), Some(format!("readme {i}").into()), 20 - i)).collect();
        let readmes: Vec<(&str, Option<serde_json::Value>, i64)> = readmes.iter().map(|(v, r, d)| (v.as_str(), r.clone(), *d)).collect();
        let packages = Arc::new(FakePackages::new());
        let repository_id = Uuid::new_v4();
        let name = NpmPackageName::parse("widget").unwrap();
        let package = NpmPackage { id: Uuid::new_v4(), package_repository_id: repository_id, name: name.clone(), created_at: Utc::now(), updated_at: Utc::now(), metadata_fetched_at: None, cached_metadata: None };
        packages.create_package(&package).await.unwrap();
        for (version, readme, days_ago) in &readmes {
            packages
                .insert_version(&NpmPackageVersion {
                    id: Uuid::new_v4(),
                    npm_package_id: package.id,
                    version: NpmVersion::parse(version).unwrap(),
                    manifest: serde_json::json!({ "readme": readme }),
                    shasum: format!("sha-{version}"),
                    integrity: "i".to_string(),
                    tarball_storage_key: "k".to_string(),
                    tarball_size_bytes: 1,
                    deprecated: false,
                    deprecated_message: None,
                    published_by: None,
                    published_at: Utc::now() - Duration::days(*days_ago),
                    origin: NpmPackageOrigin::Local,
                })
                .await
                .unwrap();
        }

        let use_case = GetNpmPackageDetailsUseCase::new(packages.clone(), Arc::new(UppercaseRenderer), Arc::new(WeeklyDownloads(0)));
        let details = use_case.execute(repository_id, &name).await.unwrap().unwrap();

        assert_eq!(details.versions.len(), 20);
        assert_eq!(details.versions[0].shasum, "sha-1.0.19");
        assert_eq!(details.readme_html.as_deref(), Some("<p>README 19</p>"));
        assert_eq!(packages.manifests_read.load(std::sync::atomic::Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn an_images_tags_are_sized_without_a_manifest_lookup_per_tag() {
        let manifests = Arc::new(FakeDockerManifestRepository::new());
        let repository_id = Uuid::new_v4();
        let name = DockerImageName::parse("my-app").unwrap();
        for (tag, size) in [("1.0", 100), ("2.0", 200), ("3.0", 300)] {
            let body = format!(r#"{{"config":{{"size":0}},"layers":[{{"size":{size}}}]}}"#).into_bytes();
            let manifest = DockerManifest { id: Uuid::new_v4(), package_repository_id: repository_id, image_name: name.clone(), digest: Digest::of(&body), media_type: DockerMediaType::DockerV2Manifest, body, created_at: Utc::now() };
            manifests.insert_manifest(&manifest, &[]).await.unwrap();
            manifests.set_tag(repository_id, &name, tag, manifest.id).await.unwrap();
        }

        let details = GetDockerImageDetailsUseCase::new(manifests.clone(), Arc::new(WeeklyDownloads(0))).execute(repository_id, &name).await.unwrap();

        assert_eq!(details.tags.iter().map(|t| t.size_bytes).collect::<Vec<_>>(), vec![Some(100), Some(200), Some(300)]);
        assert_eq!(manifests.digest_lookups.load(std::sync::atomic::Ordering::Relaxed), 0);
    }

    async fn image_with_tags(count: usize, body_of: impl Fn(usize) -> Vec<u8>) -> (Arc<FakeDockerManifestRepository>, Uuid, DockerImageName) {
        let manifests = Arc::new(FakeDockerManifestRepository::new());
        let repository_id = Uuid::new_v4();
        let name = DockerImageName::parse("my-app").unwrap();
        for i in 0..count {
            let body = body_of(i);
            let manifest = DockerManifest { id: Uuid::new_v4(), package_repository_id: repository_id, image_name: name.clone(), digest: Digest::of(&body), media_type: DockerMediaType::DockerV2Manifest, body, created_at: Utc::now() };
            manifests.insert_manifest(&manifest, &[]).await.unwrap();
            manifests.set_tag(repository_id, &name, &format!("t{i:03}"), manifest.id).await.unwrap();
        }
        (manifests, repository_id, name)
    }

    #[tokio::test]
    async fn an_image_with_300_tags_lists_the_newest_100_and_reads_only_their_manifests() {
        let (manifests, repository_id, name) = image_with_tags(300, |i| format!(r#"{{"config":{{"size":0}},"layers":[{{"size":{i}}}]}}"#).into_bytes()).await;

        let details = GetDockerImageDetailsUseCase::new(manifests.clone(), Arc::new(WeeklyDownloads(0))).execute(repository_id, &name).await.unwrap();

        assert!(details.truncated);
        assert_eq!(details.tags.len(), MAX_TAGS_IN_DETAILS);
        assert_eq!(details.tags.first().map(|t| t.tag.as_str()), Some("t200"), "the newest hundred, listed by name");
        assert_eq!(details.tags.last().map(|t| t.tag.as_str()), Some("t299"));
        assert_eq!(details.tags[0].size_bytes, Some(200));
        assert_eq!(manifests.bodies_read.load(std::sync::atomic::Ordering::Relaxed), MAX_TAGS_IN_DETAILS);
    }

    #[tokio::test]
    async fn an_image_with_few_tags_is_not_truncated_and_an_oversized_manifest_just_has_no_size() {
        let big = format!(r#"{{"config":{{"size":0}},"layers":[{{"size":5}}],"annotations":{{"pad":"{}"}}}}"#, "x".repeat(300 * 1024)).into_bytes();
        let (manifests, repository_id, name) = image_with_tags(2, |i| if i == 0 { big.clone() } else { br#"{"config":{"size":1},"layers":[{"size":2}]}"#.to_vec() }).await;

        let details = GetDockerImageDetailsUseCase::new(manifests, Arc::new(WeeklyDownloads(0))).execute(repository_id, &name).await.unwrap();

        assert!(!details.truncated);
        assert_eq!(details.tags.iter().map(|t| (t.tag.as_str(), t.size_bytes)).collect::<Vec<_>>(), vec![("t000", None), ("t001", Some(3))]);
    }

    #[test]
    fn an_image_manifests_size_is_its_config_plus_its_layers() {
        let body = br#"{"schemaVersion":2,"config":{"size":100,"digest":"sha256:a"},"layers":[{"size":1000},{"size":2000}]}"#;

        assert_eq!(declared_size_bytes(body), Some(3100));
    }

    #[test]
    fn a_manifest_list_or_unreadable_body_has_no_size() {
        assert_eq!(declared_size_bytes(br#"{"manifests":[{"digest":"sha256:a"}]}"#), None);
        assert_eq!(declared_size_bytes(b"not json"), None);
        assert_eq!(declared_size_bytes(br#"{"config":{"size":1},"layers":[{"size":"big"}]}"#), None);
        assert_eq!(declared_size_bytes(br#"{"layers":[]}"#), Some(0));
    }

    #[tokio::test]
    async fn each_tag_reports_the_size_its_manifest_declares() {
        let manifests = Arc::new(FakeDockerManifestRepository::new());
        let repository_id = Uuid::new_v4();
        let name = DockerImageName::parse("my-app").unwrap();
        let body = br#"{"config":{"size":10},"layers":[{"size":90}]}"#.to_vec();
        let manifest = DockerManifest { id: Uuid::new_v4(), package_repository_id: repository_id, image_name: name.clone(), digest: Digest::of(&body), media_type: DockerMediaType::DockerV2Manifest, body, created_at: Utc::now() };
        manifests.insert_manifest(&manifest, &[]).await.unwrap();
        manifests.set_tag(repository_id, &name, "1.0", manifest.id).await.unwrap();

        let details = GetDockerImageDetailsUseCase::new(manifests, Arc::new(WeeklyDownloads(42))).execute(repository_id, &name).await.unwrap();

        assert_eq!(details.tags[0].size_bytes, Some(100));
    }
}
