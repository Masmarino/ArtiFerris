use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use artiferris_domain::organization::{Organization, OrganizationRepositoryPort, PUBLIC_ORGANIZATION_ID};
use artiferris_domain::package_repository::{PackageRepositoryQueryPort, PackageRepositorySummary};
use artiferris_domain::permission::{public_repository_bypass_role, PermissionQueryPort, Role};
use uuid::Uuid;

use crate::error::ApplicationError;

/// Who is asking, as far as visibility goes.
#[derive(Debug, Clone, Copy)]
pub struct ReadableRepositoriesCaller {
    pub user_id: Uuid,
    pub organization_id: Uuid,
    pub is_super_admin: bool,
    pub is_organization_admin: bool,
}

#[derive(Debug, Clone)]
pub struct ReadableRepository {
    pub repository: PackageRepositorySummary,
    /// What the caller may do there: the stronger of any explicit grant and the implicit role their standing gives.
    pub role: Role,
    /// Whether `role` comes from a grant, ownership or admin standing rather than from the repository being public.
    pub explicit: bool,
    pub owner: Organization,
}

/// The repositories a caller sees when browsing one organization's page (`resolved_organization_id`): the rule the repository
/// list has always applied, moved out of the HTTP handler so that other features can start from the same set.
pub struct ListReadableRepositoriesUseCase {
    organizations: Arc<dyn OrganizationRepositoryPort>,
    repositories: Arc<dyn PackageRepositoryQueryPort>,
    permissions: Arc<dyn PermissionQueryPort>,
}

impl ListReadableRepositoriesUseCase {
    pub fn new(organizations: Arc<dyn OrganizationRepositoryPort>, repositories: Arc<dyn PackageRepositoryQueryPort>, permissions: Arc<dyn PermissionQueryPort>) -> Self {
        Self { organizations, repositories, permissions }
    }

    pub async fn execute(&self, caller: &ReadableRepositoriesCaller, resolved_organization_id: Uuid) -> Result<Vec<ReadableRepository>, ApplicationError> {
        // One batched lookup instead of one `find_by_id` per repository.
        let organizations: HashMap<Uuid, Organization> = self.organizations.list_all().await?.into_iter().map(|o| (o.id, o)).collect();
        let readable = |repository: PackageRepositorySummary, role: Role, explicit: bool| {
            organizations.get(&repository.organization_id).map(|owner| ReadableRepository { owner: owner.clone(), repository, role, explicit })
        };

        if caller.is_super_admin {
            // A super-admin's listing is inherently cross-organization — there is no single
            // `organization_id` to scope a query by here, unlike every branch below.
            return Ok(self.repositories.list_all().await?.into_iter().filter_map(|r| readable(r, Role::Admin, true)).collect());
        }
        // An organization admin sees every repository in their own org with Admin, regardless
        // of which domain the request came in on — so this scopes by the caller's organization, not the resolved one.
        if caller.is_organization_admin {
            return Ok(self.repositories.list_by_organization(caller.organization_id).await?.into_iter().filter_map(|r| readable(r, Role::Admin, true)).collect());
        }
        // One batched lookup instead of one `find_role` per repository.
        let roles: HashMap<Uuid, Role> = self.permissions.list_for_user(caller.user_id).await?.into_iter().collect();
        // A regular member only sees repositories in the resolved organization, even if they
        // have stray permission grants elsewhere — scoped at the database level instead of
        // filtering a full-table `list_all()` read in application code (M-21, B-7).
        let organization_repositories = self.repositories.list_by_organization(resolved_organization_id).await?;
        let mut visible: Vec<ReadableRepository> = organization_repositories
            .into_iter()
            .filter_map(|repository| {
                let role = *roles.get(&repository.id)?;
                readable(repository, role, true)
            })
            .collect();
        // Browsing the public organization doubles as a community page: every public personal
        // project shows up too, at the same implicit Read `public_repository_bypass_role` already
        // grants a public-organization member on any public repository — just applied per-repo here
        // instead of at the single-repository gate. Both the caller's own organization and the
        // viewed one are required, not redundant: gating on the viewed page alone would leak a
        // public personal project's existence, and its owner's name, to a caller from an unrelated
        // organization who then 404s on the repository itself.
        //
        // This genuinely needs every organization's repositories — a public personal project can
        // live in any organization — so it only pays for the full scan when the public
        // organization's own page is being viewed.
        if resolved_organization_id == PUBLIC_ORGANIZATION_ID {
            let all = self.repositories.list_all().await?;
            let already_shown: HashSet<Uuid> = visible.iter().map(|r| r.repository.id).collect();
            visible.extend(all.into_iter().filter(|r| !already_shown.contains(&r.id)).filter_map(|repository| {
                let bypass_role = public_repository_bypass_role(caller.organization_id, repository.is_public)?;
                organizations.get(&repository.organization_id).filter(|o| o.is_personal)?;
                // The stronger of the two wins: a caller's own explicit grant on their personal project
                // (e.g. the owner's Admin) must never be downgraded to Read just because it is also public.
                let (role, explicit) = match roles.get(&repository.id) {
                    Some(explicit) if explicit.satisfies(bypass_role) => (*explicit, true),
                    _ => (bypass_role, false),
                };
                readable(repository, role, explicit)
            }));
        }
        Ok(visible)
    }
}

#[cfg(test)]
mod tests {
    use artiferris_infrastructure::postgres::organization_repository::PostgresOrganizationRepository;
    use artiferris_infrastructure::postgres::package_repository_store::PostgresPackageRepositoryStore;
    use artiferris_infrastructure::postgres::permission_store::PostgresPermissionStore;
    use sqlx::PgPool;

    use super::*;

    const PUBLIC_ORG: &str = "00000000-0000-0000-0000-000000000001";

    async fn organization(pool: &PgPool, slug: &str, is_personal: bool) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO organizations (id, slug, display_name, is_personal) VALUES ($1, $2, $2, $3)").bind(id).bind(slug).bind(is_personal).execute(pool).await.unwrap();
        id
    }

    async fn repository(pool: &PgPool, organization: impl std::fmt::Display, name: &str, repo_type: &str, is_public: bool) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO package_repository_projections (id, organization_id, name, format, repo_type, version, is_public) VALUES ($1, $2::uuid, $3, 'npm', $4, 1, $5)")
            .bind(id)
            .bind(organization.to_string())
            .bind(name)
            .bind(repo_type)
            .bind(is_public)
            .execute(pool)
            .await
            .unwrap();
        id
    }

    async fn grant(pool: &PgPool, user: Uuid, repository: Uuid, role: &str) {
        sqlx::query("INSERT INTO permission_projections (user_id, repository_id, role, version) VALUES ($1, $2, $3, 1)").bind(user).bind(repository).bind(role).execute(pool).await.unwrap();
    }

    fn use_case(pool: &PgPool) -> ListReadableRepositoriesUseCase {
        let store = Arc::new(PostgresPackageRepositoryStore::new(pool.clone(), "test-secret".to_string()));
        ListReadableRepositoriesUseCase::new(Arc::new(PostgresOrganizationRepository::new(pool.clone())), store, Arc::new(PostgresPermissionStore::new(pool.clone())))
    }

    fn member(organization_id: Uuid) -> ReadableRepositoriesCaller {
        ReadableRepositoriesCaller { user_id: Uuid::new_v4(), organization_id, is_super_admin: false, is_organization_admin: false }
    }

    async fn listed(use_case: &ListReadableRepositoriesUseCase, caller: &ReadableRepositoriesCaller, resolved: Uuid) -> HashMap<Uuid, Role> {
        use_case.execute(caller, resolved).await.unwrap().into_iter().map(|r| (r.repository.id, r.role)).collect()
    }

    async fn explicit_flags(use_case: &ListReadableRepositoriesUseCase, caller: &ReadableRepositoriesCaller, resolved: Uuid) -> HashMap<Uuid, bool> {
        use_case.execute(caller, resolved).await.unwrap().into_iter().map(|r| (r.repository.id, r.explicit)).collect()
    }

    fn public_org_id() -> Uuid {
        Uuid::parse_str(PUBLIC_ORG).unwrap()
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_super_admin_sees_every_repository_of_every_organization_with_admin(pool: PgPool) {
        let acme = organization(&pool, "acme", false).await;
        let one = repository(&pool, acme, "one", "hosted", false).await;
        let two = repository(&pool, PUBLIC_ORG, "two", "proxy", false).await;
        let caller = ReadableRepositoriesCaller { is_super_admin: true, ..member(public_org_id()) };

        let seen = listed(&use_case(&pool), &caller, acme).await;

        assert_eq!(seen, HashMap::from([(one, Role::Admin), (two, Role::Admin)]));
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn an_organization_admin_sees_their_own_organization_with_admin_whatever_page_they_are_on(pool: PgPool) {
        let acme = organization(&pool, "acme", false).await;
        let globex = organization(&pool, "globex", false).await;
        let mine = repository(&pool, acme, "mine", "proxy", false).await;
        repository(&pool, globex, "theirs", "hosted", true).await;
        let caller = ReadableRepositoriesCaller { is_organization_admin: true, ..member(acme) };
        let use_case = use_case(&pool);

        assert_eq!(listed(&use_case, &caller, acme).await, HashMap::from([(mine, Role::Admin)]));
        assert_eq!(listed(&use_case, &caller, globex).await, HashMap::from([(mine, Role::Admin)]), "the resolved organization does not change an admin's listing");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_member_sees_only_the_repositories_they_hold_a_grant_on_in_the_resolved_organization(pool: PgPool) {
        let acme = organization(&pool, "acme", false).await;
        let granted = repository(&pool, acme, "granted", "hosted", false).await;
        let write = repository(&pool, acme, "write", "hosted", false).await;
        repository(&pool, acme, "no-grant", "hosted", false).await;
        repository(&pool, acme, "no-grant-public", "hosted", true).await;
        let caller = member(acme);
        grant(&pool, caller.user_id, granted, "read").await;
        grant(&pool, caller.user_id, write, "write").await;

        let seen = listed(&use_case(&pool), &caller, acme).await;

        assert_eq!(seen, HashMap::from([(granted, Role::Read), (write, Role::Write)]));
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_grant_in_another_organization_does_not_show_on_this_organizations_page(pool: PgPool) {
        let acme = organization(&pool, "acme", false).await;
        let globex = organization(&pool, "globex", false).await;
        let stray = repository(&pool, globex, "stray", "hosted", false).await;
        let caller = member(acme);
        grant(&pool, caller.user_id, stray, "admin").await;

        assert!(listed(&use_case(&pool), &caller, acme).await.is_empty());
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn on_the_public_page_a_public_organization_member_also_sees_public_personal_projects_at_read(pool: PgPool) {
        let alice_space = organization(&pool, "u-alice-space", true).await;
        let bob_space = organization(&pool, "u-bob-space", true).await;
        let public_project = repository(&pool, alice_space, "shared-lib", "hosted", true).await;
        repository(&pool, alice_space, "private-lib", "hosted", false).await;
        let bobs_public_project = repository(&pool, bob_space, "bobs-lib", "hosted", true).await;
        repository(&pool, PUBLIC_ORG, "public-org-repo", "hosted", true).await;
        let caller = member(public_org_id());
        grant(&pool, caller.user_id, bobs_public_project, "admin").await;

        let seen = listed(&use_case(&pool), &caller, public_org_id()).await;

        assert_eq!(
            seen,
            HashMap::from([(public_project, Role::Read), (bobs_public_project, Role::Admin)]),
            "public personal projects at Read, an explicit stronger grant kept, a private project and a plain public-organization repository not listed"
        );
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn only_the_implicit_public_read_is_flagged_as_not_explicit(pool: PgPool) {
        let space = organization(&pool, "u-alice-space", true).await;
        let plain = repository(&pool, space, "plain", "hosted", true).await;
        let granted_read = repository(&pool, space, "granted-read", "hosted", true).await;
        let granted_admin = repository(&pool, space, "granted-admin", "hosted", true).await;
        let caller = member(public_org_id());
        grant(&pool, caller.user_id, granted_read, "read").await;
        grant(&pool, caller.user_id, granted_admin, "admin").await;
        let use_case = use_case(&pool);

        assert_eq!(explicit_flags(&use_case, &caller, public_org_id()).await, HashMap::from([(plain, false), (granted_read, true), (granted_admin, true)]));

        let admin = ReadableRepositoriesCaller { is_organization_admin: true, ..member(space) };
        assert!(explicit_flags(&use_case, &admin, space).await.values().all(|explicit| *explicit));
        let super_admin = ReadableRepositoriesCaller { is_super_admin: true, ..member(public_org_id()) };
        assert!(explicit_flags(&use_case, &super_admin, space).await.values().all(|explicit| *explicit));
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_caller_from_an_unrelated_organization_never_sees_public_personal_projects(pool: PgPool) {
        let acme = organization(&pool, "acme", false).await;
        let space = organization(&pool, "u-alice-space", true).await;
        repository(&pool, space, "shared-lib", "hosted", true).await;

        let seen = listed(&use_case(&pool), &member(acme), public_org_id()).await;

        assert!(seen.is_empty(), "showing them would confirm the project exists to someone who then gets a 404 on it");
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn public_personal_projects_only_appear_on_the_public_organizations_page(pool: PgPool) {
        let acme = organization(&pool, "acme", false).await;
        let space = organization(&pool, "u-alice-space", true).await;
        repository(&pool, space, "shared-lib", "hosted", true).await;

        assert!(listed(&use_case(&pool), &member(public_org_id()), acme).await.is_empty());
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn each_entry_carries_its_owning_organization(pool: PgPool) {
        let acme = organization(&pool, "acme", false).await;
        repository(&pool, acme, "one", "hosted", false).await;
        let caller = ReadableRepositoriesCaller { is_organization_admin: true, ..member(acme) };

        let listing = use_case(&pool).execute(&caller, acme).await.unwrap();

        assert_eq!(listing.len(), 1);
        assert_eq!((listing[0].owner.id, listing[0].owner.slug.as_str()), (acme, "acme"));
    }
}
