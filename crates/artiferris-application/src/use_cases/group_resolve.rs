use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use futures::{StreamExt, TryStreamExt};
use artiferris_domain::package_repository::{PackageRepositoryQueryPort, PackageRepositorySummary, RepositoryType};
use uuid::Uuid;

use crate::error::ApplicationError;

/// Deepest chain of nested groups the traversal follows.
pub(crate) const MAX_GROUP_DEPTH: usize = 16;

/// Most repositories one traversal visits, groups included.
pub(crate) const MAX_GROUP_VISITS: usize = 512;

/// Group members looked up at the same time.
const MEMBER_LOOKUPS_IN_FLIGHT: usize = 16;

type Visited = Arc<Mutex<HashSet<Uuid>>>;

/// Tries locally, else recurses into group members. One `visited` set covers the traversal; past [`MAX_GROUP_DEPTH`] or
/// [`MAX_GROUP_VISITS`] it stops. `authorize_member` runs before descending into every member (not the top-level
/// repository); a rejected member is skipped like a missing one.
#[allow(clippy::too_many_arguments)]
pub fn resolve_in_group<'a, T, FHosted, FutHosted, FProxy, FutProxy, FNotFound, FAuthorize, FutAuthorize>(
    repositories: &'a Arc<dyn PackageRepositoryQueryPort>,
    repository_id: Uuid,
    visited: HashSet<Uuid>,
    try_hosted: FHosted,
    try_proxy: FProxy,
    not_found: FNotFound,
    authorize_member: FAuthorize,
) -> Pin<Box<dyn Future<Output = Result<Option<T>, ApplicationError>> + Send + 'a>>
where
    T: Send + 'a,
    FHosted: Fn(Uuid) -> FutHosted + Clone + Send + 'a,
    FutHosted: Future<Output = Result<Option<T>, ApplicationError>> + Send + 'a,
    FProxy: Fn(Uuid, PackageRepositorySummary) -> FutProxy + Clone + Send + 'a,
    FutProxy: Future<Output = Result<Option<T>, ApplicationError>> + Send + 'a,
    FNotFound: Fn() -> ApplicationError + Clone + Send + 'a,
    FAuthorize: Fn(&PackageRepositorySummary) -> FutAuthorize + Clone + Send + 'a,
    FutAuthorize: Future<Output = bool> + Send + 'a,
{
    Box::pin(async move {
        let visited: Visited = Arc::new(Mutex::new(visited));
        if !visited.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).insert(repository_id) {
            return Ok(None);
        }
        let repo = repositories.find_by_id(repository_id).await?.ok_or_else(&not_found)?;
        resolve_fetched(repositories, repo, visited, 0, try_hosted, try_proxy, not_found, authorize_member).await
    })
}

/// Same traversal as [`resolve_in_group`], but for an already-fetched summary — a group's member is never looked up by id twice.
#[allow(clippy::too_many_arguments)]
fn resolve_fetched<'a, T, FHosted, FutHosted, FProxy, FutProxy, FNotFound, FAuthorize, FutAuthorize>(
    repositories: &'a Arc<dyn PackageRepositoryQueryPort>,
    repo: PackageRepositorySummary,
    visited: Visited,
    depth: usize,
    try_hosted: FHosted,
    try_proxy: FProxy,
    not_found: FNotFound,
    authorize_member: FAuthorize,
) -> Pin<Box<dyn Future<Output = Result<Option<T>, ApplicationError>> + Send + 'a>>
where
    T: Send + 'a,
    FHosted: Fn(Uuid) -> FutHosted + Clone + Send + 'a,
    FutHosted: Future<Output = Result<Option<T>, ApplicationError>> + Send + 'a,
    FProxy: Fn(Uuid, PackageRepositorySummary) -> FutProxy + Clone + Send + 'a,
    FutProxy: Future<Output = Result<Option<T>, ApplicationError>> + Send + 'a,
    FNotFound: Fn() -> ApplicationError + Clone + Send + 'a,
    FAuthorize: Fn(&PackageRepositorySummary) -> FutAuthorize + Clone + Send + 'a,
    FutAuthorize: Future<Output = bool> + Send + 'a,
{
    Box::pin(async move {
        match repo.repo_type {
            RepositoryType::Hosted => try_hosted(repo.id).await,
            RepositoryType::Proxy => try_proxy(repo.id, repo).await,
            RepositoryType::Group => {
                if depth >= MAX_GROUP_DEPTH {
                    tracing::warn!(repository_id = %repo.id, "group nesting is too deep, not descending further");
                    return Ok(None);
                }
                let member_ids: Vec<Uuid> = {
                    let seen = visited.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                    repo.group_members.iter().copied().filter(|id| !seen.contains(id)).collect()
                };
                // At most the visit cap, a few at a time: a group with thousands of members must not turn into thousands of queries.
                let members: Vec<PackageRepositorySummary> = futures::stream::iter(member_ids.into_iter().take(MAX_GROUP_VISITS))
                    .map(|member_id| async move { repositories.find_by_id(member_id).await })
                    .buffered(MEMBER_LOOKUPS_IN_FLIGHT)
                    .try_collect::<Vec<_>>()
                    .await?
                    .into_iter()
                    .flatten()
                    .collect();
                // Hosted members go first regardless of stored order — otherwise a proxy member could shadow a private package (dependency confusion).
                let (hosted_first, rest): (Vec<_>, Vec<_>) = members.into_iter().partition(|m| m.repo_type == RepositoryType::Hosted);
                for member in hosted_first.into_iter().chain(rest) {
                    if !authorize_member(&member).await {
                        continue;
                    }
                    {
                        let mut seen = visited.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                        if seen.len() >= MAX_GROUP_VISITS {
                            tracing::warn!(repository_id = %repo.id, "group traversal visited too many repositories, stopping");
                            return Ok(None);
                        }
                        if !seen.insert(member.id) {
                            continue;
                        }
                    }
                    let result = resolve_fetched(
                        repositories,
                        member,
                        visited.clone(),
                        depth + 1,
                        try_hosted.clone(),
                        try_proxy.clone(),
                        not_found.clone(),
                        authorize_member.clone(),
                    )
                    .await?;
                    if result.is_some() {
                        return Ok(result);
                    }
                }
                Ok(None)
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use artiferris_domain::error::{DomainError, EventStoreError};
    use artiferris_domain::package_repository::RepositoryFormat;

    use super::*;
    use crate::use_cases::npm_test_support::FakeRepositories;

    fn hosted_repo(id: Uuid, org: Uuid) -> PackageRepositorySummary {
        PackageRepositorySummary {
            id,
            organization_id: org,
            name: format!("hosted-{id}"),
            format: RepositoryFormat::Npm,
            repo_type: RepositoryType::Hosted,
            remote_url: None,
            remote_username: None,
            remote_password: None,
            group_members: vec![],
            quota_bytes: None,
            retention_keep_last_n: None,
            is_public: false,
        }
    }

    fn proxy_repo(id: Uuid, org: Uuid) -> PackageRepositorySummary {
        PackageRepositorySummary {
            id,
            organization_id: org,
            name: format!("proxy-{id}"),
            format: RepositoryFormat::Npm,
            repo_type: RepositoryType::Proxy,
            remote_url: Some("https://registry.example/".to_string()),
            remote_username: None,
            remote_password: None,
            group_members: vec![],
            quota_bytes: None,
            retention_keep_last_n: None,
            is_public: false,
        }
    }

    fn group_repo(id: Uuid, org: Uuid, members: Vec<Uuid>) -> PackageRepositorySummary {
        PackageRepositorySummary {
            id,
            organization_id: org,
            name: format!("group-{id}"),
            format: RepositoryFormat::Npm,
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

    fn not_found_error() -> ApplicationError {
        ApplicationError::Domain(DomainError::Infrastructure("repository not found".to_string()))
    }

    /// Drives `resolve_in_group` with callbacks that just tag which repository/branch handled the request, so tests can assert on the walk's outcome.
    async fn resolve(repositories: &Arc<dyn PackageRepositoryQueryPort>, repository_id: Uuid) -> Result<Option<String>, ApplicationError> {
        resolve_in_group(
            repositories,
            repository_id,
            HashSet::new(),
            |id| async move { Ok(Some(format!("hosted:{id}"))) },
            |id, _repo| async move { Ok(Some(format!("proxy:{id}"))) },
            not_found_error,
            |_repo| async { true },
        )
        .await
    }

    #[tokio::test]
    async fn group_with_one_hosted_member_resolves_to_that_repository() {
        let org = Uuid::new_v4();
        let group_id = Uuid::new_v4();
        let member_id = Uuid::new_v4();
        let store = FakeRepositories::new();
        store.insert(group_repo(group_id, org, vec![member_id]));
        store.insert(hosted_repo(member_id, org));
        let repositories: Arc<dyn PackageRepositoryQueryPort> = Arc::new(store);

        let result = resolve(&repositories, group_id).await.unwrap();
        assert_eq!(result, Some(format!("hosted:{member_id}")));
    }

    /// Dependency-confusion regression: a proxy member listed before a hosted one must not shadow it.
    #[tokio::test]
    async fn a_proxy_member_listed_before_a_hosted_member_never_shadows_it() {
        let org = Uuid::new_v4();
        let group_id = Uuid::new_v4();
        let proxy_id = Uuid::new_v4();
        let hosted_id = Uuid::new_v4();
        let store = FakeRepositories::new();
        store.insert(group_repo(group_id, org, vec![proxy_id, hosted_id]));
        store.insert(proxy_repo(proxy_id, org));
        store.insert(hosted_repo(hosted_id, org));
        let repositories: Arc<dyn PackageRepositoryQueryPort> = Arc::new(store);

        let result = resolve(&repositories, group_id).await.unwrap();

        assert_eq!(result, Some(format!("hosted:{hosted_id}")), "the hosted member must win regardless of stored order");
    }

    #[tokio::test]
    async fn group_with_a_nested_group_resolves_two_levels_deep() {
        let org = Uuid::new_v4();
        let outer_group = Uuid::new_v4();
        let inner_group = Uuid::new_v4();
        let hosted_id = Uuid::new_v4();
        let store = FakeRepositories::new();
        store.insert(group_repo(outer_group, org, vec![inner_group]));
        store.insert(group_repo(inner_group, org, vec![hosted_id]));
        store.insert(hosted_repo(hosted_id, org));
        let repositories: Arc<dyn PackageRepositoryQueryPort> = Arc::new(store);

        let result = resolve(&repositories, outer_group).await.unwrap();
        assert_eq!(result, Some(format!("hosted:{hosted_id}")));
    }

    #[tokio::test]
    async fn group_that_contains_itself_is_caught_by_the_cycle_guard() {
        let org = Uuid::new_v4();
        let group_id = Uuid::new_v4();
        let store = FakeRepositories::new();
        store.insert(group_repo(group_id, org, vec![group_id]));
        let repositories: Arc<dyn PackageRepositoryQueryPort> = Arc::new(store);

        // Caught by `visited` and yields "not found" (`None`), not infinite recursion.
        let result = resolve(&repositories, group_id).await.unwrap();
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn two_groups_referencing_each_other_are_caught_by_the_cycle_guard() {
        let org = Uuid::new_v4();
        let group_a = Uuid::new_v4();
        let group_b = Uuid::new_v4();
        let store = FakeRepositories::new();
        store.insert(group_repo(group_a, org, vec![group_b]));
        store.insert(group_repo(group_b, org, vec![group_a]));
        let repositories: Arc<dyn PackageRepositoryQueryPort> = Arc::new(store);

        let result = resolve(&repositories, group_a).await.unwrap();
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn empty_group_resolves_to_none_rather_than_an_error() {
        let org = Uuid::new_v4();
        let group_id = Uuid::new_v4();
        let store = FakeRepositories::new();
        store.insert(group_repo(group_id, org, vec![]));
        let repositories: Arc<dyn PackageRepositoryQueryPort> = Arc::new(store);

        let result = resolve(&repositories, group_id).await;
        assert_eq!(result.unwrap(), None);
    }

    #[tokio::test]
    async fn missing_repository_surfaces_the_not_found_error() {
        let store = FakeRepositories::new();
        let repositories: Arc<dyn PackageRepositoryQueryPort> = Arc::new(store);

        let err = resolve(&repositories, Uuid::new_v4()).await.unwrap_err();
        assert!(matches!(err, ApplicationError::Domain(DomainError::Infrastructure(_))), "got {err:?}");
    }

    /// Fails fast past `max_visits` lookups of the same id — turns a broken cycle guard into a clear error instead of a hang.
    struct VisitCountingRepositories {
        inner: FakeRepositories,
        visits: Mutex<HashMap<Uuid, usize>>,
        max_visits: usize,
    }

    impl VisitCountingRepositories {
        fn new(max_visits: usize) -> Self {
            Self { inner: FakeRepositories::new(), visits: Mutex::new(HashMap::new()), max_visits }
        }

        fn insert(&self, summary: PackageRepositorySummary) {
            self.inner.insert(summary);
        }
    }

    #[async_trait::async_trait]
    impl PackageRepositoryQueryPort for VisitCountingRepositories {
        async fn find_by_id(&self, id: Uuid) -> Result<Option<PackageRepositorySummary>, EventStoreError> {
            let count = {
                let mut visits = self.visits.lock().unwrap();
                let count = visits.entry(id).or_insert(0);
                *count += 1;
                *count
            };
            if count > self.max_visits {
                return Err(EventStoreError::Storage(format!("repository {id} visited {count} times (max {}) - cycle guard not working", self.max_visits)));
            }
            self.inner.find_by_id(id).await
        }

        async fn find_by_org_and_name(&self, organization_id: Uuid, name: &str) -> Result<Option<PackageRepositorySummary>, EventStoreError> {
            self.inner.find_by_org_and_name(organization_id, name).await
        }

        async fn list_all(&self) -> Result<Vec<PackageRepositorySummary>, EventStoreError> {
            self.inner.list_all().await
        }

        async fn list_by_organization(&self, organization_id: Uuid) -> Result<Vec<PackageRepositorySummary>, EventStoreError> {
            self.inner.list_by_organization(organization_id).await
        }
    }

    #[tokio::test]
    async fn cycle_guard_never_looks_up_the_same_repository_more_than_once() {
        let org = Uuid::new_v4();
        let group_a = Uuid::new_v4();
        let group_b = Uuid::new_v4();
        let store = VisitCountingRepositories::new(1);
        store.insert(group_repo(group_a, org, vec![group_b]));
        store.insert(group_repo(group_b, org, vec![group_a]));
        let repositories: Arc<dyn PackageRepositoryQueryPort> = Arc::new(store);

        let result = resolve(&repositories, group_a).await;
        assert_eq!(result.unwrap(), None);
    }

    /// A member id with no repository is skipped, not a failure.
    #[tokio::test]
    async fn a_group_resolves_through_its_other_members_when_one_member_no_longer_exists() {
        let store = FakeRepositories::new();
        let org = Uuid::new_v4();
        let vanished_member_id = Uuid::new_v4(); // deliberately never inserted — simulates a soft-deleted member
        let hosted_id = Uuid::new_v4();
        store.insert(hosted_repo(hosted_id, org));
        let group_id = Uuid::new_v4();
        store.insert(group_repo(group_id, org, vec![vanished_member_id, hosted_id]));
        let repositories: Arc<dyn PackageRepositoryQueryPort> = Arc::new(store);

        let result = resolve_in_group(
            &repositories,
            group_id,
            HashSet::new(),
            |id| async move { Ok(Some(id)) },
            |_id, _repo| async { Ok(None) },
            || ApplicationError::NpmPackageNotFound,
            |_repo| async { true },
        )
        .await
        .unwrap();

        assert_eq!(result, Some(hosted_id), "the vanished member must be skipped, not fail the whole group");
    }

    /// `authorize_member` is consulted per member; a rejected member is skipped.
    #[tokio::test]
    async fn a_member_the_caller_is_not_authorized_to_read_is_skipped_not_hard_failed() {
        let store = FakeRepositories::new();
        let org = Uuid::new_v4();
        let forbidden_member_id = Uuid::new_v4();
        let allowed_member_id = Uuid::new_v4();
        store.insert(hosted_repo(forbidden_member_id, org));
        store.insert(hosted_repo(allowed_member_id, org));
        let group_id = Uuid::new_v4();
        store.insert(group_repo(group_id, org, vec![forbidden_member_id, allowed_member_id]));
        let repositories: Arc<dyn PackageRepositoryQueryPort> = Arc::new(store);

        let result = resolve_in_group(
            &repositories,
            group_id,
            HashSet::new(),
            |id| async move { Ok(Some(id)) },
            |_id, _repo| async { Ok(None) },
            || ApplicationError::NpmPackageNotFound,
            move |repo: &PackageRepositorySummary| {
                let allowed = repo.id == allowed_member_id;
                async move { allowed }
            },
        )
        .await
        .unwrap();

        assert_eq!(result, Some(allowed_member_id), "a member the caller can't read must be skipped, the search continues into the rest of the group");
    }

    /// `levels` layers of two groups, each listing both groups of the next layer, ending in one hosted repository.
    fn diamond_ladder(levels: usize) -> (FakeRepositories, Uuid, Vec<Uuid>) {
        let org = Uuid::new_v4();
        let store = FakeRepositories::new();
        let hosted_id = Uuid::new_v4();
        store.insert(hosted_repo(hosted_id, org));
        let mut next_layer = vec![hosted_id];
        let mut all = vec![hosted_id];
        for _ in 0..levels {
            let layer = vec![Uuid::new_v4(), Uuid::new_v4()];
            for &id in &layer {
                store.insert(group_repo(id, org, next_layer.clone()));
            }
            all.extend(&layer);
            next_layer = layer;
        }
        let top = Uuid::new_v4();
        store.insert(group_repo(top, org, next_layer));
        all.push(top);
        (store, top, all)
    }

    #[tokio::test]
    async fn a_diamond_of_nested_groups_costs_one_visit_per_repository_not_one_per_path() {
        let (store, top, all) = diamond_ladder(12);
        let repositories: Arc<dyn PackageRepositoryQueryPort> = Arc::new(store);
        let authorized = Arc::new(Mutex::new(0usize));
        let counter = authorized.clone();

        let result = resolve_in_group(
            &repositories,
            top,
            HashSet::new(),
            |_id| async { Ok(None::<()>) },
            |_id, _repo| async { Ok(None) },
            || ApplicationError::NpmPackageNotFound,
            move |_repo: &PackageRepositorySummary| {
                *counter.lock().unwrap() += 1;
                async { true }
            },
        )
        .await
        .unwrap();

        assert_eq!(result, None);
        let calls = *authorized.lock().unwrap();
        assert!(calls <= all.len(), "{calls} authorization checks for {} repositories", all.len());
    }

    #[tokio::test]
    async fn nesting_deeper_than_the_cap_is_not_followed() {
        let org = Uuid::new_v4();
        let store = FakeRepositories::new();
        let hosted_id = Uuid::new_v4();
        store.insert(hosted_repo(hosted_id, org));
        let mut inner = hosted_id;
        for _ in 0..(MAX_GROUP_DEPTH + 4) {
            let group = Uuid::new_v4();
            store.insert(group_repo(group, org, vec![inner]));
            inner = group;
        }
        let repositories: Arc<dyn PackageRepositoryQueryPort> = Arc::new(store);

        assert_eq!(resolve(&repositories, inner).await.unwrap(), None);
    }

    #[tokio::test]
    async fn a_group_wider_than_the_visit_cap_looks_up_no_more_members_than_the_cap() {
        let org = Uuid::new_v4();
        let store = VisitCountingRepositories::new(1);
        let members: Vec<Uuid> = (0..MAX_GROUP_VISITS + 200).map(|_| Uuid::new_v4()).collect();
        for &id in &members {
            store.insert(hosted_repo(id, org));
        }
        let group = Uuid::new_v4();
        store.insert(group_repo(group, org, members));
        let store = Arc::new(store);
        let repositories: Arc<dyn PackageRepositoryQueryPort> = store.clone();

        resolve_in_group(&repositories, group, HashSet::new(), |_id| async { Ok(None::<()>) }, |_id, _repo| async { Ok(None) }, || ApplicationError::NpmPackageNotFound, |_repo: &PackageRepositorySummary| async { true })
            .await
            .unwrap();

        let looked_up = store.visits.lock().unwrap().len();
        assert!(looked_up <= MAX_GROUP_VISITS + 1, "{looked_up} repositories looked up");
    }

    #[tokio::test]
    async fn a_group_wider_than_the_visit_cap_stops_early() {
        let org = Uuid::new_v4();
        let store = FakeRepositories::new();
        let members: Vec<Uuid> = (0..MAX_GROUP_VISITS + 50).map(|_| Uuid::new_v4()).collect();
        for &id in &members {
            store.insert(proxy_repo(id, org));
        }
        let group = Uuid::new_v4();
        store.insert(group_repo(group, org, members));
        let repositories: Arc<dyn PackageRepositoryQueryPort> = Arc::new(store);
        let tried = Arc::new(Mutex::new(0usize));
        let counter = tried.clone();

        let result = resolve_in_group(
            &repositories,
            group,
            HashSet::new(),
            |_id| async { Ok(None::<()>) },
            move |_id, _repo| {
                *counter.lock().unwrap() += 1;
                async { Ok(None) }
            },
            || ApplicationError::NpmPackageNotFound,
            |_repo: &PackageRepositorySummary| async { true },
        )
        .await
        .unwrap();

        assert_eq!(result, None);
        assert!(*tried.lock().unwrap() < MAX_GROUP_VISITS, "visited {} members", tried.lock().unwrap());
    }
}
