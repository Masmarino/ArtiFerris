use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use artiferris_domain::docker_registry::{DockerImageName, DockerManifestRepositoryPort};
use artiferris_domain::package_repository::{PackageRepositoryQueryPort, PackageRepositorySummary, RepositoryFormat, RepositoryType};
use artiferris_domain::permission::PermissionQueryPort;
use uuid::Uuid;

use crate::error::ApplicationError;
use crate::use_cases::group_resolve::{MAX_GROUP_DEPTH, MAX_GROUP_VISITS};

/// Most tags one page of `tags/list` carries, and how many it carries when the client doesn't say.
pub const MAX_TAGS_PER_PAGE: usize = 1000;

/// One page of an image's tags, in name order.
pub struct TagPage {
    pub tags: Vec<String>,
    /// Tags come after this page.
    pub has_more: bool,
}

/// A `Group` repository stores no manifests of its own — `manifests.list_tags` against its id
/// always returns empty (M-18). `ListTagsUseCase` therefore needs to know the repository's
/// `repo_type` and, for a Group, recurse into its members and UNION their tags rather than query
/// the group id directly.
pub struct ListTagsUseCase {
    manifests: Arc<dyn DockerManifestRepositoryPort>,
    repositories: Arc<dyn PackageRepositoryQueryPort>,
}

impl ListTagsUseCase {
    pub fn new(manifests: Arc<dyn DockerManifestRepositoryPort>, repositories: Arc<dyn PackageRepositoryQueryPort>) -> Self {
        Self { manifests, repositories }
    }

    pub async fn execute<FAuthorize, FutAuthorize>(
        &self,
        repository_id: Uuid,
        image_name: &DockerImageName,
        authorize_member: FAuthorize,
    ) -> Result<Vec<String>, ApplicationError>
    where
        FAuthorize: Fn(&PackageRepositorySummary) -> FutAuthorize + Clone + Send,
        FutAuthorize: Future<Output = bool> + Send,
    {
        let visited = Arc::new(Mutex::new(HashSet::from([repository_id])));
        let Some(repo) = self.repositories.find_by_id(repository_id).await? else {
            return Ok(vec![]);
        };
        let mut tags = self.collect_tags(repo, image_name, visited, 0, authorize_member).await?;
        tags.sort();
        tags.dedup();
        Ok(tags)
    }

    /// Up to `limit` tags sorting after `last`, if given.
    pub async fn execute_page<FAuthorize, FutAuthorize>(
        &self,
        repository_id: Uuid,
        image_name: &DockerImageName,
        limit: usize,
        last: Option<&str>,
        authorize_member: FAuthorize,
    ) -> Result<TagPage, ApplicationError>
    where
        FAuthorize: Fn(&PackageRepositorySummary) -> FutAuthorize + Clone + Send,
        FutAuthorize: Future<Output = bool> + Send,
    {
        let mut tags = self.execute(repository_id, image_name, authorize_member).await?;
        if let Some(last) = last {
            tags.retain(|tag| tag.as_str() > last);
        }
        let has_more = tags.len() > limit;
        tags.truncate(limit);
        Ok(TagPage { tags, has_more })
    }

    /// Same traversal limits as `group_resolve` (one shared visited set, depth and visit caps), but
    /// unions every member's tags instead of stopping at the first hit. `authorize_member` runs for
    /// each group member, never for the top-level repository (the caller checked that already).
    fn collect_tags<'a, FAuthorize, FutAuthorize>(
        &'a self,
        repo: PackageRepositorySummary,
        image_name: &'a DockerImageName,
        visited: Arc<Mutex<HashSet<Uuid>>>,
        depth: usize,
        authorize_member: FAuthorize,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<String>, ApplicationError>> + Send + 'a>>
    where
        FAuthorize: Fn(&PackageRepositorySummary) -> FutAuthorize + Clone + Send + 'a,
        FutAuthorize: Future<Output = bool> + Send + 'a,
    {
        Box::pin(async move {
            match repo.repo_type {
                RepositoryType::Hosted => Ok(self.manifests.list_tags(repo.id, image_name).await?),
                // A proxy caches manifests on demand, it has no browsable tag list.
                RepositoryType::Proxy => Ok(vec![]),
                RepositoryType::Group => {
                    if depth >= MAX_GROUP_DEPTH {
                        tracing::warn!(repository_id = %repo.id, "group nesting is too deep, not descending further");
                        return Ok(vec![]);
                    }
                    let mut all_tags = Vec::new();
                    for member_id in repo.group_members.iter().copied() {
                        {
                            let seen = visited.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                            if seen.len() >= MAX_GROUP_VISITS {
                                tracing::warn!(repository_id = %repo.id, "group traversal visited too many repositories, stopping");
                                break;
                            }
                            if seen.contains(&member_id) {
                                continue;
                            }
                        }
                        let Some(member) = self.repositories.find_by_id(member_id).await? else { continue };
                        if !authorize_member(&member).await {
                            continue;
                        }
                        if !visited.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).insert(member.id) {
                            continue;
                        }
                        all_tags.extend(self.collect_tags(member, image_name, visited.clone(), depth + 1, authorize_member.clone()).await?);
                    }
                    Ok(all_tags)
                }
            }
        })
    }
}

pub struct ListCatalogUseCase {
    manifests: Arc<dyn DockerManifestRepositoryPort>,
}

impl ListCatalogUseCase {
    pub fn new(manifests: Arc<dyn DockerManifestRepositoryPort>) -> Self {
        Self { manifests }
    }

    pub async fn execute(&self, repository_id: Uuid) -> Result<Vec<DockerImageName>, ApplicationError> {
        let names = self.manifests.list_repository_image_names(repository_id).await?;
        // Deduplicate image names defensively — callers must not assume the port returns unique names.
        let unique_names: HashSet<String> = names.into_iter().map(|n| n.as_str().to_string()).collect();
        let mut result: Vec<DockerImageName> = unique_names
            .into_iter()
            .map(|n| DockerImageName::parse(&n))
            .collect::<Result<Vec<_>, _>>()?;
        result.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        Ok(result)
    }
}

/// The `_catalog` endpoint's listing, across every readable Docker repository.
pub struct ListDockerRegistryCatalogUseCase {
    repositories: Arc<dyn PackageRepositoryQueryPort>,
    permissions: Arc<dyn PermissionQueryPort>,
    manifests: Arc<dyn DockerManifestRepositoryPort>,
}

impl ListDockerRegistryCatalogUseCase {
    pub fn new(repositories: Arc<dyn PackageRepositoryQueryPort>, permissions: Arc<dyn PermissionQueryPort>, manifests: Arc<dyn DockerManifestRepositoryPort>) -> Self {
        Self { repositories, permissions, manifests }
    }

    /// Repository names are only unique per-organization, so without `organization_id` the
    /// catalog would leak cross-organization repository existence.
    ///
    /// `is_super_admin` and each repository's `is_public` are the same bypasses every other
    /// Docker route honors (B-11) — unlike `artiferris-api`'s `AuthUser`, `DockerAuthUser` (a
    /// snapshot baked into the access token at issuance) carries no live org-admin flag, so
    /// there is no org-admin bypass to apply at this layer.
    pub async fn execute(&self, organization_id: Uuid, user_id: Uuid, is_super_admin: bool) -> Result<Vec<String>, ApplicationError> {
        // Scoped at the database level instead of filtering a full-table `list_all()` read in
        // application code (M-21, B-7).
        let docker_repos: Vec<_> =
            self.repositories.list_by_organization(organization_id).await?.into_iter().filter(|r| r.format == RepositoryFormat::Docker).collect();
        let readable: Vec<_> = if is_super_admin {
            docker_repos
        } else {
            // One batched role lookup instead of one `find_role` per repository.
            let readable_repo_ids: std::collections::HashSet<Uuid> = self.permissions.list_for_user(user_id).await?.into_iter().map(|(id, _)| id).collect();
            docker_repos.into_iter().filter(|r| r.is_public || readable_repo_ids.contains(&r.id)).collect()
        };
        let readable_ids: Vec<Uuid> = readable.iter().map(|r| r.id).collect();

        let pairs = self.manifests.list_image_names_for_repositories(&readable_ids).await?;
        let repo_names: std::collections::HashMap<Uuid, &str> = readable.iter().map(|r| (r.id, r.name.as_str())).collect();
        let mut names: Vec<String> =
            pairs.into_iter().filter_map(|(repo_id, image_name)| repo_names.get(&repo_id).map(|name| format!("{name}/{}", image_name.as_str()))).collect();
        names.sort();
        Ok(names)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::use_cases::docker_test_support::{FakeDockerManifestRepository, FakeRepositories};
    use async_trait::async_trait;
    use artiferris_domain::docker_registry::{DockerImageName, DockerManifest, DockerMediaType, Digest};
    use artiferris_domain::error::EventStoreError;
    use artiferris_domain::package_repository::{PackageRepositorySummary, RepositoryType};
    use artiferris_domain::permission::Role;

    struct FakePermissions {
        entries: Vec<(Uuid, Uuid, Role)>,
        fail_list_for_user: bool,
    }

    impl FakePermissions {
        fn new(entries: Vec<(Uuid, Uuid, Role)>) -> Self {
            Self { entries, fail_list_for_user: false }
        }
    }

    #[async_trait]
    impl PermissionQueryPort for FakePermissions {
        async fn find_role(&self, user_id: Uuid, repository_id: Uuid) -> Result<Option<Role>, EventStoreError> {
            Ok(self.entries.iter().find(|(u, r, _)| *u == user_id && *r == repository_id).map(|(_, _, role)| *role))
        }
        async fn list_for_repository(&self, repository_id: Uuid) -> Result<Vec<(Uuid, Role)>, EventStoreError> {
            Ok(self.entries.iter().filter(|(_, r, _)| *r == repository_id).map(|(u, _, role)| (*u, *role)).collect())
        }
        async fn list_for_user(&self, user_id: Uuid) -> Result<Vec<(Uuid, Role)>, EventStoreError> {
            if self.fail_list_for_user {
                return Err(EventStoreError::Storage("simulated lookup failure".to_string()));
            }
            Ok(self.entries.iter().filter(|(u, _, _)| *u == user_id).map(|(_, r, role)| (*r, *role)).collect())
        }
        async fn list_all(&self) -> Result<Vec<(Uuid, Uuid, Role)>, EventStoreError> {
            unreachable!("not exercised by this use case's tests")
        }
        async fn count_all(&self) -> Result<usize, EventStoreError> {
            unreachable!("not exercised by this use case's tests")
        }
        async fn count_for_repositories(&self, _repository_ids: &[Uuid]) -> Result<usize, EventStoreError> {
            unreachable!("not exercised by this use case's tests")
        }
    }

    /// Counts `find_by_id` calls so a runaway traversal fails instead of hanging.
    struct CountingRepositories {
        inner: FakeRepositories,
        lookups: std::sync::atomic::AtomicUsize,
    }

    #[async_trait]
    impl PackageRepositoryQueryPort for CountingRepositories {
        async fn find_by_id(&self, id: Uuid) -> Result<Option<PackageRepositorySummary>, artiferris_domain::error::EventStoreError> {
            self.lookups.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.inner.find_by_id(id).await
        }
        async fn find_by_org_and_name(&self, organization_id: Uuid, name: &str) -> Result<Option<PackageRepositorySummary>, artiferris_domain::error::EventStoreError> {
            self.inner.find_by_org_and_name(organization_id, name).await
        }
        async fn list_all(&self) -> Result<Vec<PackageRepositorySummary>, artiferris_domain::error::EventStoreError> {
            self.inner.list_all().await
        }
        async fn list_by_organization(&self, organization_id: Uuid) -> Result<Vec<PackageRepositorySummary>, artiferris_domain::error::EventStoreError> {
            self.inner.list_by_organization(organization_id).await
        }
    }

    #[tokio::test]
    async fn a_diamond_of_nested_groups_is_listed_with_one_lookup_per_repository() {
        let org = Uuid::new_v4();
        let counting = Arc::new(CountingRepositories { inner: FakeRepositories::new(), lookups: std::sync::atomic::AtomicUsize::new(0) });
        let manifests = Arc::new(FakeDockerManifestRepository::new());
        let image_name = DockerImageName::parse("myimage").unwrap();
        let hosted_id = Uuid::new_v4();
        counting.inner.insert(docker_repo(hosted_id, org, "hosted"));
        let m = manifest(hosted_id, &image_name);
        manifests.insert_manifest(&m, &[]).await.unwrap();
        manifests.set_tag(hosted_id, &image_name, "v1", m.id).await.unwrap();
        let mut next_layer = vec![hosted_id];
        // 12 layers keep the depth under the cap while giving thousands of paths.
        for _ in 0..12 {
            let layer = vec![Uuid::new_v4(), Uuid::new_v4()];
            for &id in &layer {
                counting.inner.insert(group_repo(id, org, next_layer.clone()));
            }
            next_layer = layer;
        }
        let top = Uuid::new_v4();
        counting.inner.insert(group_repo(top, org, next_layer));

        let use_case = ListTagsUseCase::new(manifests, counting.clone());
        let tags = use_case.execute(top, &image_name, |_repo: &PackageRepositorySummary| async { true }).await.unwrap();

        assert_eq!(tags, vec!["v1".to_string()]);
        let lookups = counting.lookups.load(std::sync::atomic::Ordering::SeqCst);
        assert!(lookups <= 40, "{lookups} repository lookups for 26 repositories");
    }

    #[tokio::test]
    async fn tags_are_listed_a_page_at_a_time_in_name_order() {
        let manifests = Arc::new(FakeDockerManifestRepository::new());
        let repositories = Arc::new(FakeRepositories::new());
        let image_name = DockerImageName::parse("myimage").unwrap();
        let hosted_id = Uuid::new_v4();
        repositories.insert(docker_repo(hosted_id, Uuid::new_v4(), "hosted"));
        let m = manifest(hosted_id, &image_name);
        manifests.insert_manifest(&m, &[]).await.unwrap();
        for tag in ["e", "b", "d", "a", "c"] {
            manifests.set_tag(hosted_id, &image_name, tag, m.id).await.unwrap();
        }
        let use_case = ListTagsUseCase::new(manifests, repositories);
        let allow = |_repo: &PackageRepositorySummary| async { true };

        let first = use_case.execute_page(hosted_id, &image_name, 2, None, allow).await.unwrap();
        let second = use_case.execute_page(hosted_id, &image_name, 2, Some("b"), allow).await.unwrap();
        let last = use_case.execute_page(hosted_id, &image_name, 2, Some("d"), allow).await.unwrap();
        let none = use_case.execute_page(hosted_id, &image_name, 0, None, allow).await.unwrap();

        assert_eq!((first.tags, first.has_more), (vec!["a".to_string(), "b".to_string()], true));
        assert_eq!((second.tags, second.has_more), (vec!["c".to_string(), "d".to_string()], true));
        assert_eq!((last.tags, last.has_more), (vec!["e".to_string()], false));
        assert!(none.tags.is_empty());
    }

    #[tokio::test]
    async fn nesting_deeper_than_the_cap_is_not_followed_when_listing_tags() {
        let manifests = Arc::new(FakeDockerManifestRepository::new());
        let repositories = Arc::new(FakeRepositories::new());
        let org = Uuid::new_v4();
        let image_name = DockerImageName::parse("myimage").unwrap();
        let hosted_id = Uuid::new_v4();
        repositories.insert(docker_repo(hosted_id, org, "hosted"));
        let m = manifest(hosted_id, &image_name);
        manifests.insert_manifest(&m, &[]).await.unwrap();
        manifests.set_tag(hosted_id, &image_name, "deep", m.id).await.unwrap();
        let mut inner = hosted_id;
        for _ in 0..(MAX_GROUP_DEPTH + 4) {
            let group = Uuid::new_v4();
            repositories.insert(group_repo(group, org, vec![inner]));
            inner = group;
        }

        let use_case = ListTagsUseCase::new(manifests, repositories);
        let tags = use_case.execute(inner, &image_name, |_repo: &PackageRepositorySummary| async { true }).await.unwrap();

        assert!(tags.is_empty());
    }

    #[tokio::test]
    async fn a_group_wider_than_the_visit_cap_stops_early_when_listing_tags() {
        let org = Uuid::new_v4();
        let counting = Arc::new(CountingRepositories { inner: FakeRepositories::new(), lookups: std::sync::atomic::AtomicUsize::new(0) });
        let members: Vec<Uuid> = (0..MAX_GROUP_VISITS + 50).map(|_| Uuid::new_v4()).collect();
        for &id in &members {
            counting.inner.insert(docker_repo(id, org, &format!("member-{id}")));
        }
        let group = Uuid::new_v4();
        counting.inner.insert(group_repo(group, org, members));

        let use_case = ListTagsUseCase::new(Arc::new(FakeDockerManifestRepository::new()), counting.clone());
        let image_name = DockerImageName::parse("myimage").unwrap();
        use_case.execute(group, &image_name, |_repo: &PackageRepositorySummary| async { true }).await.unwrap();

        let lookups = counting.lookups.load(std::sync::atomic::Ordering::SeqCst);
        assert!(lookups <= MAX_GROUP_VISITS + 1, "{lookups} lookups");
    }

    fn docker_repo(id: Uuid, organization_id: Uuid, name: &str) -> PackageRepositorySummary {
        docker_repo_with_visibility(id, organization_id, name, false)
    }

    fn docker_repo_with_visibility(id: Uuid, organization_id: Uuid, name: &str, is_public: bool) -> PackageRepositorySummary {
        PackageRepositorySummary {
            id,
            organization_id,
            name: name.to_string(),
            format: RepositoryFormat::Docker,
            repo_type: RepositoryType::Hosted,
            remote_url: None,
            remote_username: None,
            remote_password: None,
            group_members: vec![],
            quota_bytes: None,
            retention_keep_last_n: None,
            is_public,
        }
    }

    fn manifest(repository_id: Uuid, name: &DockerImageName) -> DockerManifest {
        DockerManifest {
            id: Uuid::new_v4(), package_repository_id: repository_id, image_name: name.clone(),
            digest: Digest::of(name.as_str().as_bytes()), media_type: DockerMediaType::DockerV2Manifest,
            body: b"{}".to_vec(), created_at: chrono::Utc::now(),
        }
    }

    #[tokio::test]
    async fn lists_tags_for_one_image_sorted_and_ignores_other_images() {
        let manifests = Arc::new(FakeDockerManifestRepository::new());
        let repositories = Arc::new(FakeRepositories::new());
        let repository_id = Uuid::new_v4();
        repositories.insert(docker_repo(repository_id, Uuid::new_v4(), "repo"));
        let name = DockerImageName::parse("myimage").unwrap();
        let other_name = DockerImageName::parse("other").unwrap();
        let m = manifest(repository_id, &name);
        manifests.insert_manifest(&m, &[]).await.unwrap();
        manifests.set_tag(repository_id, &name, "v2", m.id).await.unwrap();
        manifests.set_tag(repository_id, &name, "v1", m.id).await.unwrap();
        let other = manifest(repository_id, &other_name);
        manifests.insert_manifest(&other, &[]).await.unwrap();
        manifests.set_tag(repository_id, &other_name, "latest", other.id).await.unwrap();

        let use_case = ListTagsUseCase::new(manifests, repositories);
        let tags = use_case.execute(repository_id, &name, |_repo: &PackageRepositorySummary| async { true }).await.unwrap();

        assert_eq!(tags, vec!["v1".to_string(), "v2".to_string()]);
    }

    /// M-18: a Group repository stores no manifests of its own — listing tags on it must aggregate
    /// (union) every hosted member's tags instead of querying the group id directly and getting an
    /// empty list back.
    #[tokio::test]
    async fn listing_tags_on_a_group_repository_aggregates_its_hosted_members() {
        let manifests = Arc::new(FakeDockerManifestRepository::new());
        let repositories = Arc::new(FakeRepositories::new());
        let org = Uuid::new_v4();
        let member_a = Uuid::new_v4();
        let member_b = Uuid::new_v4();
        repositories.insert(docker_repo(member_a, org, "member-a"));
        repositories.insert(docker_repo(member_b, org, "member-b"));
        let group_id = Uuid::new_v4();
        repositories.insert(group_repo(group_id, org, vec![member_a, member_b]));
        let image_name = DockerImageName::parse("myimage").unwrap();
        let manifest_a = manifest(member_a, &image_name);
        manifests.insert_manifest(&manifest_a, &[]).await.unwrap();
        manifests.set_tag(member_a, &image_name, "v1", manifest_a.id).await.unwrap();
        let manifest_b = manifest(member_b, &image_name);
        manifests.insert_manifest(&manifest_b, &[]).await.unwrap();
        manifests.set_tag(member_b, &image_name, "v2", manifest_b.id).await.unwrap();

        let use_case = ListTagsUseCase::new(manifests, repositories);
        let tags = use_case.execute(group_id, &image_name, |_repo: &PackageRepositorySummary| async { true }).await.unwrap();

        assert_eq!(tags, vec!["v1".to_string(), "v2".to_string()]);
    }

    /// A group containing itself (directly) must be caught by the cycle guard and terminate with an
    /// empty aggregate rather than recursing forever.
    #[tokio::test]
    async fn a_group_that_contains_itself_terminates_instead_of_looping() {
        let manifests = Arc::new(FakeDockerManifestRepository::new());
        let repositories = Arc::new(FakeRepositories::new());
        let org = Uuid::new_v4();
        let group_id = Uuid::new_v4();
        repositories.insert(group_repo(group_id, org, vec![group_id]));

        let use_case = ListTagsUseCase::new(manifests, repositories);
        let image_name = DockerImageName::parse("myimage").unwrap();
        let tags = use_case.execute(group_id, &image_name, |_repo: &PackageRepositorySummary| async { true }).await.unwrap();

        assert!(tags.is_empty());
    }

    /// Two groups referencing each other transitively must also be caught, not just the direct
    /// self-reference case.
    #[tokio::test]
    async fn two_groups_referencing_each_other_terminate_instead_of_looping() {
        let manifests = Arc::new(FakeDockerManifestRepository::new());
        let repositories = Arc::new(FakeRepositories::new());
        let org = Uuid::new_v4();
        let group_a = Uuid::new_v4();
        let group_b = Uuid::new_v4();
        repositories.insert(group_repo(group_a, org, vec![group_b]));
        repositories.insert(group_repo(group_b, org, vec![group_a]));

        let use_case = ListTagsUseCase::new(manifests, repositories);
        let image_name = DockerImageName::parse("myimage").unwrap();
        let tags = use_case.execute(group_a, &image_name, |_repo: &PackageRepositorySummary| async { true }).await.unwrap();

        assert!(tags.is_empty());
    }

    /// C-1 parity: a caller authorized to read the Group itself must not automatically see tags from
    /// a member they individually lack access to — `authorize_member` is re-checked per member.
    #[tokio::test]
    async fn a_member_the_caller_is_not_authorized_to_read_is_excluded_from_the_aggregate() {
        let manifests = Arc::new(FakeDockerManifestRepository::new());
        let repositories = Arc::new(FakeRepositories::new());
        let org = Uuid::new_v4();
        let readable_member = Uuid::new_v4();
        let forbidden_member = Uuid::new_v4();
        repositories.insert(docker_repo(readable_member, org, "readable"));
        repositories.insert(docker_repo(forbidden_member, org, "forbidden"));
        let group_id = Uuid::new_v4();
        repositories.insert(group_repo(group_id, org, vec![readable_member, forbidden_member]));
        let image_name = DockerImageName::parse("myimage").unwrap();
        let manifest_readable = manifest(readable_member, &image_name);
        manifests.insert_manifest(&manifest_readable, &[]).await.unwrap();
        manifests.set_tag(readable_member, &image_name, "v1", manifest_readable.id).await.unwrap();
        let manifest_forbidden = manifest(forbidden_member, &image_name);
        manifests.insert_manifest(&manifest_forbidden, &[]).await.unwrap();
        manifests.set_tag(forbidden_member, &image_name, "v2", manifest_forbidden.id).await.unwrap();

        let use_case = ListTagsUseCase::new(manifests, repositories);
        let tags = use_case
            .execute(group_id, &image_name, move |repo: &PackageRepositorySummary| {
                let allowed = repo.id == readable_member;
                async move { allowed }
            })
            .await
            .unwrap();

        assert_eq!(tags, vec!["v1".to_string()], "the forbidden member's tag must not leak into the aggregate");
    }

    fn group_repo(id: Uuid, organization_id: Uuid, members: Vec<Uuid>) -> PackageRepositorySummary {
        PackageRepositorySummary {
            id,
            organization_id,
            name: format!("group-{id}"),
            format: RepositoryFormat::Docker,
            repo_type: RepositoryType::Group,
            remote_url: None,
            remote_username: None,
            remote_password: None,
            group_members: members,
            quota_bytes: None,
            retention_keep_last_n: None,
            is_public: false,
        }
    }

    #[tokio::test]
    async fn lists_catalog_image_names_sorted_and_deduplicated() {
        let manifests = Arc::new(FakeDockerManifestRepository::new());
        let repository_id = Uuid::new_v4();
        let name = DockerImageName::parse("myimage").unwrap();
        let m = manifest(repository_id, &name);
        manifests.insert_manifest(&m, &[]).await.unwrap();
        // Two tags on the SAME image must still surface it only once.
        manifests.set_tag(repository_id, &name, "v1", m.id).await.unwrap();
        manifests.set_tag(repository_id, &name, "v2", m.id).await.unwrap();

        let use_case = ListCatalogUseCase::new(manifests);
        let names = use_case.execute(repository_id).await.unwrap();

        assert_eq!(names, vec![name]);
    }

    #[tokio::test]
    async fn registry_catalog_excludes_a_readable_repository_from_a_different_organization() {
        let manifests = Arc::new(FakeDockerManifestRepository::new());
        let queried_org = Uuid::new_v4();
        let other_org = Uuid::new_v4();
        let in_queried_org = Uuid::new_v4();
        let in_other_org = Uuid::new_v4();
        let repositories = Arc::new(FakeRepositories::new());
        repositories.insert(docker_repo(in_queried_org, queried_org, "mine"));
        repositories.insert(docker_repo(in_other_org, other_org, "not-mine"));

        let name = DockerImageName::parse("myimage").unwrap();
        let mine_manifest = manifest(in_queried_org, &name);
        manifests.insert_manifest(&mine_manifest, &[]).await.unwrap();
        manifests.set_tag(in_queried_org, &name, "latest", mine_manifest.id).await.unwrap();
        let other_manifest = manifest(in_other_org, &name);
        manifests.insert_manifest(&other_manifest, &[]).await.unwrap();
        manifests.set_tag(in_other_org, &name, "latest", other_manifest.id).await.unwrap();

        let user_id = Uuid::new_v4();
        // Readable in BOTH repositories — proves the organization filter, not the
        // permission filter, is what excludes the other organization's repository.
        let permissions = Arc::new(FakePermissions::new(vec![(user_id, in_queried_org, Role::Read), (user_id, in_other_org, Role::Read)]));
        let use_case = ListDockerRegistryCatalogUseCase::new(repositories, permissions, manifests);

        let names = use_case.execute(queried_org, user_id, false).await.unwrap();

        assert_eq!(names, vec!["mine/myimage".to_string()]);
    }

    #[tokio::test]
    async fn registry_catalog_lists_only_repositories_the_caller_can_read() {
        let manifests = Arc::new(FakeDockerManifestRepository::new());
        let organization_id = Uuid::new_v4();
        let readable_id = Uuid::new_v4();
        let unreadable_id = Uuid::new_v4();
        let repositories = Arc::new(FakeRepositories::new());
        repositories.insert(docker_repo(readable_id, organization_id, "readable"));
        repositories.insert(docker_repo(unreadable_id, organization_id, "unreadable"));

        let name = DockerImageName::parse("myimage").unwrap();
        let readable_manifest = manifest(readable_id, &name);
        manifests.insert_manifest(&readable_manifest, &[]).await.unwrap();
        manifests.set_tag(readable_id, &name, "latest", readable_manifest.id).await.unwrap();
        let unreadable_manifest = manifest(unreadable_id, &name);
        manifests.insert_manifest(&unreadable_manifest, &[]).await.unwrap();
        manifests.set_tag(unreadable_id, &name, "latest", unreadable_manifest.id).await.unwrap();

        let user_id = Uuid::new_v4();
        let permissions = Arc::new(FakePermissions::new(vec![(user_id, readable_id, Role::Read)]));
        let use_case = ListDockerRegistryCatalogUseCase::new(repositories, permissions, manifests);

        let names = use_case.execute(organization_id, user_id, false).await.unwrap();

        assert_eq!(names, vec!["readable/myimage".to_string()]);
    }

    #[tokio::test]
    async fn registry_catalog_batches_multiple_readable_repositories_without_cross_contamination() {
        let manifests = Arc::new(FakeDockerManifestRepository::new());
        let organization_id = Uuid::new_v4();
        let repo_a = Uuid::new_v4();
        let repo_b = Uuid::new_v4();
        let repositories = Arc::new(FakeRepositories::new());
        repositories.insert(docker_repo(repo_a, organization_id, "alpha"));
        repositories.insert(docker_repo(repo_b, organization_id, "beta"));

        let name = DockerImageName::parse("myimage").unwrap();
        let manifest_a = manifest(repo_a, &name);
        manifests.insert_manifest(&manifest_a, &[]).await.unwrap();
        manifests.set_tag(repo_a, &name, "latest", manifest_a.id).await.unwrap();
        let manifest_b = manifest(repo_b, &name);
        manifests.insert_manifest(&manifest_b, &[]).await.unwrap();
        manifests.set_tag(repo_b, &name, "latest", manifest_b.id).await.unwrap();

        let user_id = Uuid::new_v4();
        let permissions = Arc::new(FakePermissions::new(vec![(user_id, repo_a, Role::Read), (user_id, repo_b, Role::Read)]));
        let use_case = ListDockerRegistryCatalogUseCase::new(repositories, permissions, manifests);

        let names = use_case.execute(organization_id, user_id, false).await.unwrap();

        assert_eq!(names, vec!["alpha/myimage".to_string(), "beta/myimage".to_string()], "the single batched query must attribute each image to its own repository");
    }

    #[tokio::test]
    async fn registry_catalog_is_empty_for_a_user_with_no_readable_repositories() {
        let manifests = Arc::new(FakeDockerManifestRepository::new());
        let organization_id = Uuid::new_v4();
        let repository_id = Uuid::new_v4();
        let repositories = Arc::new(FakeRepositories::new());
        repositories.insert(docker_repo(repository_id, organization_id, "some-repo"));
        let name = DockerImageName::parse("myimage").unwrap();
        let m = manifest(repository_id, &name);
        manifests.insert_manifest(&m, &[]).await.unwrap();
        manifests.set_tag(repository_id, &name, "latest", m.id).await.unwrap();

        let permissions = Arc::new(FakePermissions::new(vec![]));
        let use_case = ListDockerRegistryCatalogUseCase::new(repositories, permissions, manifests);

        assert!(use_case.execute(organization_id, Uuid::new_v4(), false).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn registry_catalog_includes_a_public_repository_with_no_explicit_grant() {
        let manifests = Arc::new(FakeDockerManifestRepository::new());
        let organization_id = Uuid::new_v4();
        let repository_id = Uuid::new_v4();
        let repositories = Arc::new(FakeRepositories::new());
        repositories.insert(docker_repo_with_visibility(repository_id, organization_id, "public-repo", true));
        let name = DockerImageName::parse("myimage").unwrap();
        let m = manifest(repository_id, &name);
        manifests.insert_manifest(&m, &[]).await.unwrap();
        manifests.set_tag(repository_id, &name, "latest", m.id).await.unwrap();

        // No explicit grants at all — visibility must come purely from `is_public`.
        let permissions = Arc::new(FakePermissions::new(vec![]));
        let use_case = ListDockerRegistryCatalogUseCase::new(repositories, permissions, manifests);

        let names = use_case.execute(organization_id, Uuid::new_v4(), false).await.unwrap();

        assert_eq!(names, vec!["public-repo/myimage".to_string()]);
    }

    #[tokio::test]
    async fn registry_catalog_includes_every_repository_for_a_super_admin() {
        let manifests = Arc::new(FakeDockerManifestRepository::new());
        let organization_id = Uuid::new_v4();
        let repository_id = Uuid::new_v4();
        let repositories = Arc::new(FakeRepositories::new());
        repositories.insert(docker_repo(repository_id, organization_id, "private-repo"));
        let name = DockerImageName::parse("myimage").unwrap();
        let m = manifest(repository_id, &name);
        manifests.insert_manifest(&m, &[]).await.unwrap();
        manifests.set_tag(repository_id, &name, "latest", m.id).await.unwrap();

        // No explicit grants at all — a super-admin must still see everything.
        let permissions = Arc::new(FakePermissions::new(vec![]));
        let use_case = ListDockerRegistryCatalogUseCase::new(repositories, permissions, manifests);

        let names = use_case.execute(organization_id, Uuid::new_v4(), true).await.unwrap();

        assert_eq!(names, vec!["private-repo/myimage".to_string()]);
    }

    #[tokio::test]
    async fn a_failed_permission_lookup_surfaces_as_an_error_instead_of_a_silently_incomplete_catalog() {
        let manifests = Arc::new(FakeDockerManifestRepository::new());
        let organization_id = Uuid::new_v4();
        let repository_id = Uuid::new_v4();
        let repositories = Arc::new(FakeRepositories::new());
        repositories.insert(docker_repo(repository_id, organization_id, "repo"));

        let mut permissions = FakePermissions::new(vec![]);
        permissions.fail_list_for_user = true;
        let use_case = ListDockerRegistryCatalogUseCase::new(repositories, Arc::new(permissions), manifests);

        assert!(use_case.execute(organization_id, Uuid::new_v4(), false).await.is_err(), "a lookup failure must surface as an error, not an empty or partial catalog");
    }
}
