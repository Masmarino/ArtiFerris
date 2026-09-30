use std::sync::Arc;

use artiferris_domain::audit::{EventPublisherPort, NpmPackageEvent};
use artiferris_domain::npm_package::{NpmPackageName, NpmPackageRepositoryPort, NpmVersion, UnpublishVersionOutcome};
use artiferris_domain::storage::StorageBackendPort;
use uuid::Uuid;

use crate::error::ApplicationError;

pub struct UnpublishNpmPackageUseCase {
    packages: Arc<dyn NpmPackageRepositoryPort>,
    storage: Arc<dyn StorageBackendPort>,
    events: Arc<dyn EventPublisherPort>,
}

impl UnpublishNpmPackageUseCase {
    pub fn new(packages: Arc<dyn NpmPackageRepositoryPort>, storage: Arc<dyn StorageBackendPort>, events: Arc<dyn EventPublisherPort>) -> Self {
        Self { packages, storage, events }
    }

    pub async fn execute_version(
        &self,
        repository_id: Uuid,
        name: &NpmPackageName,
        version: &NpmVersion,
        actor_id: Uuid,
    ) -> Result<(), ApplicationError> {
        // The row first: a crash then leaves a stray file, never a version without its tarball.
        let removed = match self.packages.unpublish_version(repository_id, name, version).await? {
            UnpublishVersionOutcome::PackageNotFound => return Err(ApplicationError::NpmPackageNotFound),
            UnpublishVersionOutcome::VersionNotFound => return Err(ApplicationError::NpmVersionNotFound),
            UnpublishVersionOutcome::Removed(removed) => removed,
        };
        self.delete_tarball(repository_id, &removed.tarball_storage_key).await;

        self.events
            .publish_npm_event(
                NpmPackageEvent::PackageVersionUnpublished { package_name: name.as_str().to_string(), version: version.as_str() },
                removed.package_id,
                repository_id,
                Some(actor_id),
            )
            .await?;

        if removed.package_deleted {
            self.events
                .publish_npm_event(NpmPackageEvent::PackageDeleted { package_name: name.as_str().to_string() }, removed.package_id, repository_id, Some(actor_id))
                .await?;
        }

        Ok(())
    }

    pub async fn execute_whole_package(&self, repository_id: Uuid, name: &NpmPackageName, actor_id: Uuid) -> Result<(), ApplicationError> {
        let removed = self.packages.unpublish_package(repository_id, name).await?.ok_or(ApplicationError::NpmPackageNotFound)?;
        for storage_key in &removed.tarball_storage_keys {
            self.delete_tarball(repository_id, storage_key).await;
        }
        self.events
            .publish_npm_event(NpmPackageEvent::PackageDeleted { package_name: name.as_str().to_string() }, removed.package_id, repository_id, Some(actor_id))
            .await?;
        Ok(())
    }

    /// Best effort: a file left behind is only wasted space.
    async fn delete_tarball(&self, repository_id: Uuid, storage_key: &str) {
        if let Err(e) = self.storage.delete(repository_id, storage_key).await {
            tracing::warn!(%repository_id, storage_key, error = %e, "could not remove the tarball of an unpublished version");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::use_cases::npm_test_support::{FakeEvents, FakePackages, FakeStorage};
    use artiferris_domain::npm_package::{NpmPackage, NpmPackageName, NpmPackageOrigin, NpmPackageVersion, NpmVersion, NpmPackageRepositoryPort};
    use artiferris_domain::storage::StorageBackendPort;
    use uuid::Uuid;
    use chrono::Utc;
    use std::sync::Arc;

    async fn seed_one_version(packages: &FakePackages, storage: &FakeStorage, repository_id: Uuid, name: &NpmPackageName, version: &NpmVersion) -> Uuid {
        let package = NpmPackage { id: Uuid::new_v4(), package_repository_id: repository_id, name: name.clone(), created_at: Utc::now(), updated_at: Utc::now(), metadata_fetched_at: None, cached_metadata: None };
        packages.create_package(&package).await.unwrap();
        storage.write(repository_id, "left-pad/-/left-pad-1.0.0.tgz", b"bytes").await.unwrap();
        packages.insert_version(&NpmPackageVersion {
            id: Uuid::new_v4(), npm_package_id: package.id, version: version.clone(), manifest: serde_json::json!({}),
            shasum: "s".into(), integrity: "i".into(), tarball_storage_key: "left-pad/-/left-pad-1.0.0.tgz".into(),
            tarball_size_bytes: 5, deprecated: false, deprecated_message: None, published_by: None, published_at: Utc::now(),
            origin: NpmPackageOrigin::Local,
        }).await.unwrap();
        package.id
    }

    #[tokio::test]
    async fn unpublishing_the_only_version_deletes_the_whole_package() {
        let packages = Arc::new(FakePackages::new());
        let storage = Arc::new(FakeStorage::new());
        let events = Arc::new(FakeEvents::new());
        let repository_id = Uuid::new_v4();
        let name = NpmPackageName::parse("left-pad").unwrap();
        let version = NpmVersion::parse("1.0.0").unwrap();
        let actor_id = Uuid::new_v4();
        seed_one_version(&packages, &storage, repository_id, &name, &version).await;

        assert!(storage.written.lock().unwrap().contains_key(&(repository_id, "left-pad/-/left-pad-1.0.0.tgz".to_string())));

        let use_case = UnpublishNpmPackageUseCase::new(packages.clone(), storage.clone(), events.clone());
        use_case.execute_version(repository_id, &name, &version, actor_id).await.unwrap();

        assert!(packages.find_package(repository_id, &name).await.unwrap().is_none());

        assert!(!storage.written.lock().unwrap().contains_key(&(repository_id, "left-pad/-/left-pad-1.0.0.tgz".to_string())));
        assert!(storage.written.lock().unwrap().is_empty(), "storage should be empty after unpublishing the last version");

        let published_events = events.npm_events.lock().unwrap();
        assert_eq!(published_events.len(), 2, "expected exactly 2 events for unpublishing the last version");

        match &published_events[0].0 {
            NpmPackageEvent::PackageVersionUnpublished { package_name, version: v } => {
                assert_eq!(package_name, "left-pad");
                assert_eq!(v, "1.0.0");
            }
            _ => panic!("first event should be PackageVersionUnpublished"),
        }

        match &published_events[1].0 {
            NpmPackageEvent::PackageDeleted { package_name } => {
                assert_eq!(package_name, "left-pad");
            }
            _ => panic!("second event should be PackageDeleted"),
        }

        assert_eq!(published_events[0].3, Some(actor_id));
        assert_eq!(published_events[1].3, Some(actor_id));
        assert!(published_events.iter().all(|event| event.2 == repository_id), "events are filed under the repository they happened in");
    }

    #[tokio::test]
    async fn unpublishing_a_missing_version_is_an_error() {
        let use_case = UnpublishNpmPackageUseCase::new(Arc::new(FakePackages::new()), Arc::new(FakeStorage::new()), Arc::new(FakeEvents::new()));
        let result = use_case.execute_version(Uuid::new_v4(), &NpmPackageName::parse("left-pad").unwrap(), &NpmVersion::parse("1.0.0").unwrap(), Uuid::new_v4()).await;
        assert!(matches!(result, Err(crate::error::ApplicationError::NpmPackageNotFound)));
    }

    #[tokio::test]
    async fn unpublishing_the_version_a_dist_tag_points_at_removes_the_dangling_tag() {
        let packages = Arc::new(FakePackages::new());
        let storage = Arc::new(FakeStorage::new());
        let events = Arc::new(FakeEvents::new());
        let repository_id = Uuid::new_v4();
        let name = NpmPackageName::parse("left-pad").unwrap();
        let v1 = NpmVersion::parse("1.0.0").unwrap();
        let v2 = NpmVersion::parse("2.0.0").unwrap();
        let actor_id = Uuid::new_v4();

        let package_id = seed_one_version(&packages, &storage, repository_id, &name, &v1).await;
        storage.write(repository_id, "left-pad/-/left-pad-2.0.0.tgz", b"bytes").await.unwrap();
        packages.insert_version(&NpmPackageVersion {
            id: Uuid::new_v4(), npm_package_id: package_id, version: v2.clone(), manifest: serde_json::json!({}),
            shasum: "s2".into(), integrity: "i2".into(), tarball_storage_key: "left-pad/-/left-pad-2.0.0.tgz".into(),
            tarball_size_bytes: 5, deprecated: false, deprecated_message: None, published_by: None, published_at: Utc::now(),
            origin: NpmPackageOrigin::Local,
        }).await.unwrap();

        packages.set_dist_tag(package_id, "latest", &v2).await.unwrap();
        assert_eq!(packages.list_dist_tags(package_id).await.unwrap().len(), 1);

        let use_case = UnpublishNpmPackageUseCase::new(packages.clone(), storage.clone(), events.clone());
        use_case.execute_version(repository_id, &name, &v2, actor_id).await.unwrap();

        let remaining_tags = packages.list_dist_tags(package_id).await.unwrap();
        assert!(
            remaining_tags.iter().all(|t| t.tag != "latest"),
            "dangling `latest` dist-tag should have been removed when the version it pointed at was unpublished, found: {remaining_tags:?}"
        );

        assert!(packages.find_package(repository_id, &name).await.unwrap().is_some());
        assert!(packages.find_version(package_id, &v1).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn a_tarball_that_cannot_be_deleted_never_leaves_a_version_without_its_file() {
        let packages = Arc::new(FakePackages::new());
        let storage = Arc::new(FakeStorage::new());
        let repository_id = Uuid::new_v4();
        let name = NpmPackageName::parse("left-pad").unwrap();
        let version = NpmVersion::parse("1.0.0").unwrap();
        let package_id = seed_one_version(&packages, &storage, repository_id, &name, &version).await;
        *storage.fail_deletes.lock().unwrap() = true;

        let use_case = UnpublishNpmPackageUseCase::new(packages.clone(), storage.clone(), Arc::new(FakeEvents::new()));
        use_case.execute_version(repository_id, &name, &version, Uuid::new_v4()).await.unwrap();

        assert!(packages.find_version(package_id, &version).await.unwrap().is_none(), "the version row is gone even though the file could not be removed");
    }

    #[tokio::test]
    async fn unpublished_versions_are_remembered_one_by_one_and_for_a_whole_package() {
        let packages = Arc::new(FakePackages::new());
        let storage = Arc::new(FakeStorage::new());
        let repository_id = Uuid::new_v4();
        let name = NpmPackageName::parse("left-pad").unwrap();
        let v1 = NpmVersion::parse("1.0.0").unwrap();
        let v2 = NpmVersion::parse("2.0.0").unwrap();
        let package_id = seed_one_version(&packages, &storage, repository_id, &name, &v1).await;
        packages.insert_version(&NpmPackageVersion {
            id: Uuid::new_v4(), npm_package_id: package_id, version: v2.clone(), manifest: serde_json::json!({}),
            shasum: "s2".into(), integrity: "i2".into(), tarball_storage_key: "left-pad/-/left-pad-2.0.0.tgz".into(),
            tarball_size_bytes: 5, deprecated: false, deprecated_message: None, published_by: None, published_at: Utc::now(),
            origin: NpmPackageOrigin::Local,
        }).await.unwrap();
        let use_case = UnpublishNpmPackageUseCase::new(packages.clone(), storage, Arc::new(FakeEvents::new()));

        use_case.execute_version(repository_id, &name, &v1, Uuid::new_v4()).await.unwrap();
        assert!(packages.was_unpublished(repository_id, &name, &v1).await.unwrap());
        assert!(!packages.was_unpublished(repository_id, &name, &v2).await.unwrap());

        use_case.execute_whole_package(repository_id, &name, Uuid::new_v4()).await.unwrap();
        assert!(packages.was_unpublished(repository_id, &name, &v2).await.unwrap());
    }
}
