use axum::http::StatusCode;
use artiferris_domain::audit::SecurityEvent;
use artiferris_domain::package_repository::PackageRepositorySummary;
use artiferris_domain::permission::{Role, organization_admin_bypass_role, public_repository_read_bypass};
use uuid::Uuid;

use crate::auth_middleware::AuthUser;
use crate::state::AppState;

impl artiferris_application::authz_primitives::OrganizationScoped for AuthUser {
    fn is_super_admin(&self) -> bool {
        self.is_super_admin
    }
    fn organization_id(&self) -> Uuid {
        self.organization_id
    }
}

/// Shared by `effective_repository_role` and `management_repository_role`: super-admin shortcut, repository fetch,
/// organization-admin bypass and the live explicit-grant lookup. Only `effective_repository_role` adds the
/// public-organization bypass: "public" unlocks reading package content, not the permission roster.
async fn base_repository_role(state: &AppState, user: &AuthUser, repository_id: Uuid) -> Result<(Option<PackageRepositorySummary>, Option<Role>), StatusCode> {
    if user.is_super_admin {
        return Ok((None, Some(Role::Admin)));
    }

    let repo = state.repositories.find_by_id(repository_id).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let organization_admin_role =
        repo.as_ref().filter(|_| user.is_organization_admin).and_then(|r| organization_admin_bypass_role(user.is_organization_admin, user.organization_id, r.organization_id));
    if let Some(role) = organization_admin_role {
        return Ok((repo, Some(role)));
    }

    let explicit_role = state.permissions.find_role(user.id, repository_id).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok((repo, explicit_role))
}

/// The caller's effective role on a repository: `Admin` for a super-admin or that organization's admin, `Read` for
/// anyone on a public repository, otherwise the explicit grant. The single source of truth: every authz check and
/// `my_role` response goes through it.
pub async fn effective_repository_role(state: &AppState, user: &AuthUser, repository_id: Uuid) -> Result<Option<Role>, StatusCode> {
    Ok(repository_roles(state, user, repository_id).await?.effective)
}

/// The effective role plus the part of it that does not come from the repository being public.
pub struct RepositoryRoles {
    pub effective: Option<Role>,
    /// Super-admin, organization admin or an explicit grant; `None` for a caller who only has the implicit public `Read`.
    pub explicit: Option<Role>,
}

pub async fn repository_roles(state: &AppState, user: &AuthUser, repository_id: Uuid) -> Result<RepositoryRoles, StatusCode> {
    let (repo, explicit) = base_repository_role(state, user, repository_id).await?;
    if user.is_super_admin {
        return Ok(RepositoryRoles { effective: explicit, explicit });
    }

    // Unlike the organization-admin bypass (always Admin), this one is only Read: it must not shadow a higher explicit
    // grant, so take the stronger of the two.
    let public_role = repo.as_ref().and_then(|r| public_repository_read_bypass(r.is_public));
    let effective = match (explicit, public_role) {
        (Some(explicit), Some(public)) => Some(if explicit.satisfies(public) { explicit } else { public }),
        (Some(explicit), None) => Some(explicit),
        (None, public) => public,
    };
    Ok(RepositoryRoles { effective, explicit })
}

/// Like `effective_repository_role`, but never grants through the public-organization bypass: a member of the public
/// organization must not enumerate a repository's permission roster because it is `is_public`.
async fn management_repository_role(state: &AppState, user: &AuthUser, repository_id: Uuid) -> Result<Option<Role>, StatusCode> {
    let (_repo, role) = base_repository_role(state, user, repository_id).await?;
    Ok(role)
}

/// The `list_permissions` counterpart of `require_repository_access`, built on `management_repository_role`: a
/// public-organization member with no grant is rejected the same on a public repository as on a private one.
pub async fn require_management_access(
    state: &AppState,
    user: &AuthUser,
    repository_organization_id: Uuid,
    repository_id: Uuid,
    minimum_role: Role,
    action: &str,
) -> Result<(), StatusCode> {
    if require_same_organization(user, repository_organization_id).is_err() {
        let organization = state.organizations.find_by_id(repository_organization_id).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let personal_grant_satisfies = organization.is_some_and(|org| org.is_personal)
            && management_repository_role(state, user, repository_id).await?.is_some_and(|role| role.satisfies(minimum_role));
        if !personal_grant_satisfies {
            return Err(StatusCode::NOT_FOUND);
        }
    }
    match management_repository_role(state, user, repository_id).await? {
        Some(role) if role.satisfies(minimum_role) => Ok(()),
        _ => {
            crate::state::record_security_event(state, SecurityEvent::AccessDenied { user_id: user.id, repository_id, action: action.to_string() }, Some(user.id)).await;
            Err(StatusCode::FORBIDDEN)
        }
    }
}

pub async fn require_repository_role(
    state: &AppState,
    user: &AuthUser,
    repository_id: Uuid,
    minimum_role: Role,
    action: &str,
) -> Result<(), StatusCode> {
    let role = effective_repository_role(state, user, repository_id).await?;

    match role {
        Some(role) if role.satisfies(minimum_role) => Ok(()),
        _ => {
            crate::state::record_security_event(state, SecurityEvent::AccessDenied { user_id: user.id, repository_id, action: action.to_string() }, Some(user.id)).await;
            Err(StatusCode::FORBIDDEN)
        }
    }
}

pub fn require_super_admin(user: &AuthUser) -> Result<(), StatusCode> {
    if user.is_super_admin {
        Ok(())
    } else {
        Err(StatusCode::FORBIDDEN)
    }
}

pub fn require_same_organization(user: &AuthUser, organization_id: Uuid) -> Result<(), StatusCode> {
    artiferris_application::authz_primitives::require_same_organization(user, organization_id).map_err(|e| match e {
        artiferris_application::authz_primitives::RepositoryAccessError::NotFound => StatusCode::NOT_FOUND,
        artiferris_application::authz_primitives::RepositoryAccessError::MethodNotAllowed => StatusCode::METHOD_NOT_ALLOWED,
        artiferris_application::authz_primitives::RepositoryAccessError::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    })
}

/// Super-admin, or that organization's own admin — the authorization shape shared by every "an organization configures its own X" route (branding, identity provider config).
pub fn require_organization_admin(user: &AuthUser, organization_id: Uuid) -> Result<(), StatusCode> {
    require_super_admin(user).or_else(|_| {
        require_same_organization(user, organization_id)?;
        if user.is_organization_admin {
            Ok(())
        } else {
            Err(StatusCode::FORBIDDEN)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use artiferris_domain::package_repository::{RepositoryFormat, RepositoryType};
    use chrono::Utc;

    fn user(is_super_admin: bool, is_organization_admin: bool, organization_id: Uuid) -> AuthUser {
        AuthUser { id: Uuid::new_v4(), username: "test".to_string(), is_super_admin, is_organization_admin, organization_id, created_at: Utc::now() }
    }

    fn test_config() -> Config {
        Config {
            database_url: String::new(),
            jwt_secret: "test-secret".to_string(),
            secrets_encryption_key: "test-secrets-encryption-key".to_string(),
            storage_root: std::env::temp_dir().to_string_lossy().to_string(),
            bind_addr: "0.0.0.0:0".to_string(),
            cors_allowed_origin: None,
            docker_token_realm_override: None,
            public_url: "http://localhost:4200".to_string(),
            db_max_connections: artiferris_infrastructure::postgres::DEFAULT_DB_MAX_CONNECTIONS,
            artiferris_base_domain: "artiferris.localhost".to_string(),
            trusted_proxy_ips: std::collections::HashSet::new(),
            audit_retention_days: None,
        }
    }

    /// A caller from an organization other than the repository's (and other than the public one) still gets implicit
    /// `Read` on a public repository, matching what the npm and Docker data planes enforce.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn effective_repository_role_grants_read_on_a_public_repo_to_a_caller_from_any_organization(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let owner_id = state.create_organization.execute("widgets-inc", "Widgets Inc").await.unwrap();
        let owner_admin_id = state.create_user.execute(owner_id, "widgets-admin", "sup3r-s3cret!", false).await.unwrap();
        let repo_id = state.create_repository.execute(owner_id, "backend", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, owner_admin_id).await.unwrap();
        state.set_repository_visibility.execute(repo_id, true, owner_admin_id).await.unwrap();

        let acme_id = state.create_organization.execute("acme", "Acme Corp").await.unwrap();
        let caller = user(false, false, acme_id);

        let role = effective_repository_role(&state, &caller, repo_id).await.unwrap();
        assert_eq!(role, Some(Role::Read), "a caller from ANY organization must get Read on a public repository, not just the public organization's own members");
    }

    #[test]
    fn a_super_admin_passes_regardless_of_organization() {
        let target_org = Uuid::new_v4();
        assert!(require_organization_admin(&user(true, false, Uuid::new_v4()), target_org).is_ok());
    }

    #[test]
    fn that_organizations_own_admin_passes() {
        let org = Uuid::new_v4();
        assert!(require_organization_admin(&user(false, true, org), org).is_ok());
    }

    #[test]
    fn a_regular_member_of_that_organization_is_rejected() {
        let org = Uuid::new_v4();
        assert!(require_organization_admin(&user(false, false, org), org).is_err());
    }

    #[test]
    fn an_admin_of_a_different_organization_is_rejected() {
        let org = Uuid::new_v4();
        assert!(require_organization_admin(&user(false, true, Uuid::new_v4()), org).is_err());
    }

    #[test]
    fn a_non_admin_of_a_different_organization_gets_not_found_not_forbidden() {
        let org = Uuid::new_v4();
        let err = require_organization_admin(&user(false, false, Uuid::new_v4()), org).unwrap_err();
        assert_eq!(err, StatusCode::NOT_FOUND);
    }
}
