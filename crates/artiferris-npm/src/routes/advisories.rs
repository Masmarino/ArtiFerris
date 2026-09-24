use std::collections::HashMap;

use axum::body::Body;
use std::net::SocketAddr;

use axum::extract::rejection::ExtensionRejection;
use axum::extract::{ConnectInfo, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::post;
use axum::{Json, Router};
use artiferris_domain::permission::Role;
use serde_json::json;

use crate::auth::NpmAuthUser;
use crate::authz::{require_npm_format_repository, require_repository_by_name, require_repository_role};
use crate::body::{BodyCaller, read_json};
use crate::errors::{bad_request, npm_error_response};
use crate::organization_resolution::ResolvedOrganization;
use crate::state::NpmState;

const ADVISORY_BODY_LIMIT_BYTES: usize = 8 * 1024 * 1024;
const ADVISORY_MEMORY_FACTOR: usize = 4;
/// Most packages, and versions of one package, relayed in one request.
pub(crate) const MAX_ADVISORY_PACKAGES: usize = 2000;
const MAX_ADVISORY_VERSIONS_PER_PACKAGE: usize = 200;

pub fn router() -> Router<NpmState> {
    Router::new().route("/{repository}/-/npm/v1/security/advisories/bulk", post(bulk_advisories))
}

/// Real `npm audit`'s own endpoint — forwards straight to npm's advisory database rather than looking anything up locally.
async fn bulk_advisories(
    State(state): State<NpmState>,
    Path(repository): Path<String>,
    headers: HeaderMap,
    connect_info: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
    resolved_org: ResolvedOrganization,
    user: NpmAuthUser,
    body: Body,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let repo = require_repository_by_name(&state, &user, resolved_org.0.id, &repository).await.map_err(|s| (s, Json(json!({ "error": "repository not found or inaccessible" }))))?;
    require_npm_format_repository(&repo).map_err(|s| (s, Json(json!({ "error": "repository not found or inaccessible" }))))?;
    require_repository_role(&state, &user, repo.id, repo.organization_id, Role::Read).await.map_err(|s| (s, Json(json!({ "error": "forbidden" }))))?;

    let caller = BodyCaller::new(&state, user.id, &headers, &connect_info);
    let (packages, _budget) = read_json::<HashMap<String, Vec<String>>>(&state, &headers, &caller, body, ADVISORY_BODY_LIMIT_BYTES, ADVISORY_MEMORY_FACTOR).await?;
    if packages.len() > MAX_ADVISORY_PACKAGES || packages.values().any(|versions| versions.len() > MAX_ADVISORY_VERSIONS_PER_PACKAGE) {
        return Err(bad_request("too many packages or versions in one advisory request"));
    }
    let result = state.bulk_audit.execute(&packages).await.map_err(npm_error_response)?;
    Ok(Json(result))
}
