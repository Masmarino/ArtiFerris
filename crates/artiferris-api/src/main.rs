mod auth_middleware;
mod authz;
mod body_timeout;
mod config;
mod dto;
mod install_location;
mod organization_middleware;
mod response_policy;
mod seo;
mod routes;
mod serve;
mod state;

use std::sync::Arc;

use axum::routing::get;
use axum::Router;
use tower_http::compression::CompressionLayer;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::trace::TraceLayer;

use crate::body_timeout::{limit_body_time, BodyTimeouts};
use crate::config::Config;
use crate::state::AppState;

/// How long a client gets to send its request headers.
const HEADER_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Well above the 2 MiB branding-upload cap, still bounds memory per `/api/*` request.
const JSON_API_BODY_LIMIT_BYTES: usize = 10 * 1024 * 1024;

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt::init();

    let config = Config::from_env();
    configure_ssrf_allowlist();
    let pool = artiferris_infrastructure::postgres::connect(&config.database_url, config.db_max_connections).await.expect("failed to connect to postgres");
    artiferris_infrastructure::postgres::run_migrations(&pool).await.expect("failed to run migrations");
    reencrypt_stored_secrets(&pool, &config.secrets_encryption_key).await;
    match artiferris_infrastructure::postgres::reserved_name_audit::find_reserved_name_conflicts(&pool).await {
        Ok(conflicts) if !conflicts.is_empty() => {
            tracing::warn!("these names predate the reserved \"artiferris-\" prefix; they keep working but should be renamed: {}", conflicts.join(", "));
        }
        Ok(_) => {}
        Err(e) => tracing::warn!("could not check for names using the reserved prefix: {e}"),
    }

    let state = AppState::build(pool.clone(), &config);
    bootstrap_super_admin(&state).await;
    spawn_metrics_snapshot_timer(&state);
    spawn_retention_sweep_timer(&state);
    spawn_upload_sweep_timer(&state);
    spawn_repository_deletion_sweep_timer(&state);
    spawn_download_flush_timer(&state);
    spawn_rate_limit_sync(&state);
    spawn_download_prune_timer(&state);
    spawn_audit_prune_timer(&state);
    let flush_downloads = state.flush_downloads.clone();
    let shutting_down = state.shutting_down.clone();
    let seo_app_state = state.clone();
    let app = build_router_with_cors(
        state,
        config.cors_allowed_origin.clone(),
        config.jwt_secret.clone(),
        config.docker_token_realm_override.clone(),
        config.public_url.clone(),
    );

    let static_dir = std::env::var("STATIC_DIR").unwrap_or_else(|_| "./static".to_string());
    let index_template = std::fs::read_to_string(format!("{static_dir}/index.html")).unwrap_or_else(|e| {
        tracing::warn!("could not read {static_dir}/index.html, pages will be served without the app: {e}");
        "<!doctype html><html><head></head><body></body></html>".to_string()
    });
    let app = with_app_fallback(app, &static_dir, seo::SeoState::new(seo_app_state, index_template), config.public_url.starts_with("https://"));

    let listener = tokio::net::TcpListener::bind(&config.bind_addr).await.expect("failed to bind");
    tracing::info!("artiferris-api listening on {}", config.bind_addr);
    spawn_audit_backfill(pool, config.db_max_connections);
    let (drain_delay, drain_timeout) = shutdown_timings();
    serve::serve(listener, app, HEADER_READ_TIMEOUT, drain_timeout, shutdown_sequence(shutting_down, drain_delay)).await;
    // The last few seconds of downloads are still in memory; write them before the process goes.
    match flush_downloads.execute().await {
        Ok(written) if written > 0 => tracing::info!(written, "flushed download counts on shutdown"),
        Ok(_) => {}
        Err(e) => tracing::warn!("could not flush download counts on shutdown: {e}"),
    }
}

/// Static files first; whatever they don't answer is an SPA route (served with its own head), a sitemap or a 404.
fn with_app_fallback(app: Router, static_dir: &str, seo: seo::SeoState, hsts_enabled: bool) -> Router {
    let app_routes = seo::app_router(static_dir, seo).layer(axum::middleware::from_fn_with_state(BodyTimeouts::default(), limit_body_time));
    // Second pass of the security headers: the fallback did not exist yet at the first one (see with_security_headers).
    with_security_headers(app.fallback_service(app_routes), hsts_enabled)
}

/// A typo stops startup: an allowlist that silently came out empty or too wide is worse than no server.
fn configure_ssrf_allowlist() {
    let raw = std::env::var("ARTIFERRIS_SSRF_ALLOWED_CIDRS").unwrap_or_default();
    let allowed = artiferris_infrastructure::ssrf_allowlist::parse_allowed_cidrs(&raw).unwrap_or_else(|e| panic!("invalid ARTIFERRIS_SSRF_ALLOWED_CIDRS: {e}"));
    if !allowed.is_empty() {
        tracing::info!("ARTIFERRIS_SSRF_ALLOWED_CIDRS exempts the listed ranges from the outbound-request guard");
    }
    artiferris_infrastructure::ssrf_allowlist::configure(allowed);
}

/// Reports what is still in the old format or under the previous key, and rewrites it only when
/// `SECRETS_REENCRYPT_LEGACY=true`. Earlier releases cannot read the result; rolling back past the migrations needs a
/// restore whatever this flag says.
async fn reencrypt_stored_secrets(pool: &sqlx::PgPool, key: &str) {
    let previous_key = std::env::var("SECRETS_ENCRYPTION_KEY_PREVIOUS").ok().filter(|s| !s.is_empty());
    if let Some(previous) = &previous_key {
        artiferris_infrastructure::secret_box::set_previous_key(previous);
    }
    let apply = matches!(std::env::var("SECRETS_REENCRYPT_LEGACY").ok().as_deref(), Some("true" | "1"));
    let report = artiferris_infrastructure::secret_migration::reencrypt_secrets(pool, key, previous_key.as_deref(), apply).await.expect("failed to re-encrypt stored secrets");
    if report.upgraded > 0 {
        tracing::info!(upgraded = report.upgraded, "re-encrypted stored secrets under the current SECRETS_ENCRYPTION_KEY; releases before the versioned secret format can no longer read them");
    }
    if report.pending > 0 {
        tracing::warn!(
            pending = report.pending,
            "stored secrets are in an old format or under SECRETS_ENCRYPTION_KEY_PREVIOUS; once the upgrade is verified, set SECRETS_REENCRYPT_LEGACY=true and restart to convert them (releases before the versioned secret format can no longer read them after that)"
        );
    }
    if report.failed > 0 {
        tracing::error!(failed = report.failed, "some stored secrets could not be read with SECRETS_ENCRYPTION_KEY (or SECRETS_ENCRYPTION_KEY_PREVIOUS); they were left as they are and the features that need them will not work until an admin enters them again");
    }
}

/// Below this many pool connections a request and a batch would compete for the last one.
const MIN_CONNECTIONS_FOR_BACKFILL: u32 = 3;

fn audit_backfill_may_run(max_connections: u32, forced: bool) -> bool {
    forced || max_connections >= MIN_CONNECTIONS_FOR_BACKFILL
}

/// Runs once the listener is up, so a big event history never delays startup.
fn spawn_audit_backfill(pool: sqlx::PgPool, max_connections: u32) {
    let forced = matches!(std::env::var("ARTIFERRIS_AUDIT_BACKFILL_FORCE").ok().as_deref(), Some("true" | "1"));
    if !audit_backfill_may_run(max_connections, forced) {
        tracing::warn!(
            max_connections,
            "not stamping the organization on older audit events: DB_MAX_CONNECTIONS is below {MIN_CONNECTIONS_FOR_BACKFILL} and the job would compete with requests for the pool. Organization audit views miss those events until DB_MAX_CONNECTIONS is raised, or ARTIFERRIS_AUDIT_BACKFILL_FORCE=true runs it anyway"
        );
        return;
    }
    tokio::spawn(async move {
        artiferris_infrastructure::postgres::audit_backfill::backfill_until_done(&pool, std::time::Duration::from_millis(50), std::time::Duration::from_secs(60)).await;
    });
}

/// Liveness (`/healthz`) says the process is up; readiness says it can reach its database.
async fn readyz(axum::extract::State(state): axum::extract::State<AppState>) -> axum::http::StatusCode {
    if !state.shutting_down.load(std::sync::atomic::Ordering::Relaxed) && state.readiness.is_ready().await {
        axum::http::StatusCode::OK
    } else {
        axum::http::StatusCode::SERVICE_UNAVAILABLE
    }
}

/// How long to keep serving after a stop was asked for, so the load balancer notices the failing readiness probe and stops
/// sending requests, and how long the requests in flight then get to finish.
const DEFAULT_SHUTDOWN_DRAIN_SECONDS: u64 = 0;
const DEFAULT_SHUTDOWN_TIMEOUT_SECONDS: u64 = 25;

/// Unset or empty keeps `default`; anything that is not a whole number of seconds is an error rather than a silent default.
fn parse_seconds(name: &str, raw: Option<&str>, default: u64) -> Result<std::time::Duration, String> {
    match raw.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(std::time::Duration::from_secs(default)),
        Some(value) => value.parse::<u64>().map(std::time::Duration::from_secs).map_err(|_| format!("{name} must be a whole number of seconds, got {value:?}")),
    }
}

fn shutdown_timings() -> (std::time::Duration, std::time::Duration) {
    let read = |name: &str, default: u64| parse_seconds(name, std::env::var(name).ok().as_deref(), default).unwrap_or_else(|message| panic!("{message}"));
    (read("ARTIFERRIS_SHUTDOWN_DRAIN_SECONDS", DEFAULT_SHUTDOWN_DRAIN_SECONDS), read("ARTIFERRIS_SHUTDOWN_TIMEOUT_SECONDS", DEFAULT_SHUTDOWN_TIMEOUT_SECONDS))
}

/// Waits for the stop signal, fails readiness at once, and keeps serving for `drain_delay` before the listener closes.
async fn shutdown_sequence(shutting_down: std::sync::Arc<std::sync::atomic::AtomicBool>, drain_delay: std::time::Duration) {
    shutdown_signal().await;
    shutting_down.store(true, std::sync::atomic::Ordering::Relaxed);
    if !drain_delay.is_zero() {
        tracing::info!(drain_seconds = drain_delay.as_secs(), "stop requested: failing readiness and serving until the load balancer has caught up");
        tokio::time::sleep(drain_delay).await;
    }
}

/// Resolves on Ctrl+C, or on SIGTERM where the platform has it (what a container runtime sends).
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {}
        () = terminate => {}
    }
}

/// No-op unless both bootstrap env vars are set and the `users` table is empty. Failures are logged, never fatal.
async fn bootstrap_super_admin(state: &AppState) {
    // docker-compose passes unset vars through as an empty string — treat empty as absent.
    let (Some(username), Some(password)) = (
        std::env::var("ARTIFERRIS_BOOTSTRAP_ADMIN_USERNAME").ok().filter(|s| !s.is_empty()),
        std::env::var("ARTIFERRIS_BOOTSTRAP_ADMIN_PASSWORD").ok().filter(|s| !s.is_empty()),
    ) else {
        return;
    };

    match state.users.list_all().await {
        Ok(users) if users.is_empty() => {
            let public_org = match state.organizations.find_public().await {
                Ok(org) => org,
                Err(e) => {
                    tracing::warn!("failed to resolve the public organization for super-admin bootstrap: {e}");
                    return;
                }
            };
            match state.create_user.execute(public_org.id, &username, &password, true).await {
                Ok(id) => tracing::info!("bootstrapped initial super-admin {username} ({id})"),
                Err(e) => tracing::warn!("failed to bootstrap initial super-admin {username}: {e}"),
            }
        }
        Ok(_) => {}
        Err(e) => tracing::warn!("failed to read users table for super-admin bootstrap: {e}"),
    }
}

/// Snapshots at startup, then once an hour: the instances take turns, so a deployment records one snapshot per hour
/// however many replicas it runs.
fn spawn_metrics_snapshot_timer(state: &AppState) {
    let record_metrics_snapshot = state.record_metrics_snapshot.clone();
    let interval = std::time::Duration::from_secs(60 * 60);
    tokio::spawn(artiferris_application::periodic::run_claimed_forever(state.periodic_jobs.clone(), "metrics-snapshot", std::time::Duration::ZERO, interval, move || {
        let record_metrics_snapshot = record_metrics_snapshot.clone();
        async move {
            if let Err(e) = record_metrics_snapshot.execute().await {
                tracing::warn!("failed to record metrics snapshot: {e}");
            }
        }
    }));
}

/// Runs every 6 hours, on one instance at a time; no immediate run on startup, unlike the metrics timer.
fn spawn_retention_sweep_timer(state: &AppState) {
    let sweep_retention = state.sweep_retention.clone();
    let interval = std::time::Duration::from_secs(6 * 60 * 60);
    tokio::spawn(artiferris_application::periodic::run_claimed_forever(state.periodic_jobs.clone(), "retention-sweep", interval, interval, move || {
        let sweep_retention = sweep_retention.clone();
        async move {
            match sweep_retention.execute().await {
                Ok(report) => {
                    if report != Default::default() {
                        tracing::info!(
                            npm_versions_deleted = report.npm_versions_deleted,
                            docker_tags_deleted = report.docker_tags_deleted,
                            docker_untagged_manifests_deleted = report.docker_untagged_manifests_deleted,
                            "retention sweep pruned old versions, tags and untagged manifests"
                        );
                    }
                }
                Err(e) => tracing::warn!("retention sweep failed: {e}"),
            }
        }
    }));
}

/// Runs hourly, not at startup, on every instance: it also removes the half-written upload files of this instance's own
/// volume, which no other instance can reach. Its database work is a set of deletes that are safe to repeat. Reclaims abandoned Docker upload sessions that the lazy sweep in
/// `DockerUploadSessionPort::find` never reaches (a truly abandoned session is never looked up again), and blobs no
/// manifest ever referenced.
fn spawn_upload_sweep_timer(state: &AppState) {
    let sweep_expired_uploads = state.sweep_expired_uploads.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60 * 60));
        interval.tick().await; // consume the immediate first tick — no run on startup
        loop {
            interval.tick().await;
            match sweep_expired_uploads.execute().await {
                Ok(report) => {
                    if report != Default::default() {
                        tracing::info!(
                            sessions = report.sessions_removed,
                            blob_links = report.blob_links_removed,
                            blobs = report.blobs_removed,
                            temp_files = report.temp_files_removed,
                            "upload sweep removed abandoned Docker upload sessions, unreferenced blobs and stale temp files"
                        );
                    }
                }
                Err(e) => tracing::warn!("upload sweep failed: {e}"),
            }
        }
    });
}

/// Runs daily, not at startup, on one instance at a time: enough for the 30-day grace period this sweep enforces.
fn spawn_repository_deletion_sweep_timer(state: &AppState) {
    let sweep_repository_deletions = state.sweep_repository_deletions.clone();
    let interval = std::time::Duration::from_secs(24 * 60 * 60);
    tokio::spawn(artiferris_application::periodic::run_claimed_forever(state.periodic_jobs.clone(), "repository-deletion-sweep", interval, interval, move || {
        let sweep_repository_deletions = sweep_repository_deletions.clone();
        async move {
            match sweep_repository_deletions.execute().await {
                Ok(removed) => {
                    if removed > 0 {
                        tracing::info!(removed, "repository deletion sweep hard-deleted repositories past their grace period");
                    }
                }
                Err(e) => tracing::warn!("repository deletion sweep failed: {e}"),
            }
        }
    }));
}

/// Moves the in-memory download counts to the database every 30 seconds; a crash loses at most that much.
fn spawn_download_flush_timer(state: &AppState) {
    let flush_downloads = state.flush_downloads.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        interval.tick().await; // consume the immediate first tick — the buffer is empty at startup
        loop {
            interval.tick().await;
            if let Err(e) = flush_downloads.execute().await {
                tracing::warn!("download count flush failed, will retry: {e}");
            }
        }
    });
}

/// Shares the anonymous request budgets with the other instances every couple of seconds. A store that does not answer
/// leaves this instance limiting on its own count; the warning is logged at most once a minute.
fn spawn_rate_limit_sync(state: &AppState) {
    let limiter = state.anonymous_limiter.clone();
    let store = state.rate_limit_store.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(artiferris_application::rate_limiter::SYNC_INTERVAL);
        let mut ticks: u64 = 0;
        let mut last_warning: Option<std::time::Instant> = None;
        loop {
            interval.tick().await;
            ticks += 1;
            let mut outcome = limiter.sync_once(store.as_ref()).await.map(|()| 0);
            if outcome.is_ok() && ticks % 15 == 0 {
                outcome = limiter.purge(store.as_ref()).await;
            }
            if let Err(e) = outcome {
                if last_warning.is_none_or(|at| at.elapsed() >= std::time::Duration::from_secs(60)) {
                    tracing::warn!("sharing the anonymous request budgets failed, limiting on this instance's own count: {e}");
                    last_warning = Some(std::time::Instant::now());
                }
            }
        }
    });
}

/// Runs daily on one instance at a time; no immediate run on startup, same as the other sweep timers.
fn spawn_download_prune_timer(state: &AppState) {
    let prune_download_stats = state.prune_download_stats.clone();
    let interval = std::time::Duration::from_secs(24 * 60 * 60);
    tokio::spawn(artiferris_application::periodic::run_claimed_forever(state.periodic_jobs.clone(), "download-stats-sweep", interval, interval, move || {
        let prune_download_stats = prune_download_stats.clone();
        async move {
            match prune_download_stats.execute().await {
                Ok(removed) if removed > 0 => tracing::info!(removed, "download stats sweep removed old daily counts"),
                Ok(_) => {}
                Err(e) => tracing::warn!("download stats sweep failed: {e}"),
            }
        }
    }));
}

/// First sweep shortly after startup, then daily, on one instance at a time.
fn spawn_audit_prune_timer(state: &AppState) {
    let prune_audit_events = state.prune_audit_events.clone();
    tokio::spawn(artiferris_application::periodic::run_claimed_forever(
        state.periodic_jobs.clone(),
        "audit-sweep",
        artiferris_application::audit_retention::first_sweep_delay(),
        artiferris_application::audit_retention::SWEEP_INTERVAL,
        move || {
            let prune_audit_events = prune_audit_events.clone();
            async move {
                match prune_audit_events.execute().await {
                    Ok(removed) if removed > 0 => tracing::info!(removed, "audit sweep removed events past the retention window"),
                    Ok(_) => {}
                    Err(e) => tracing::warn!("audit sweep failed: {e}"),
                }
            }
        },
    ));
}

/// Fully permissive CORS — the default that keeps a separately served Angular dev server working.
pub fn build_router(state: AppState) -> Router {
    build_router_with_cors(state, None, "insecure-dev-only-jwt-secret".to_string(), None, "http://localhost:4200".to_string())
}

pub fn build_router_with_cors(
    state: AppState,
    cors_allowed_origin: Option<String>,
    jwt_secret: String,
    docker_token_realm_override: Option<String>,
    public_url: String,
) -> Router {
    build_router_with_body_timeouts(state, cors_allowed_origin, jwt_secret, docker_token_realm_override, public_url, BodyTimeouts::default())
}

fn build_router_with_body_timeouts(
    state: AppState,
    cors_allowed_origin: Option<String>,
    jwt_secret: String,
    docker_token_realm_override: Option<String>,
    public_url: String,
    body_timeouts: BodyTimeouts,
) -> Router {
    // One guard for both registries, so the body-memory budget is global.
    let guard = Arc::new(artiferris_application::request_guard::RequestGuard::new(state.trusted_proxies.clone()).with_anonymous_limit(state.anonymous_limiter.clone(), state.anonymous_registry_reads_per_minute));
    let npm_state = build_npm_state(&state, &public_url, guard.clone());
    let docker_state = build_docker_state(&state, &jwt_secret, docker_token_realm_override, &public_url, guard);
    // Scoped to this JSON surface only — /npm and /v2 already serve compressed binary content.
    let json_api_routes = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/readyz", get(readyz))
        .route("/api/version", get(|| async { axum::Json(serde_json::json!({ "version": env!("CARGO_PKG_VERSION") })) }))
        .merge(routes::admin::router())
        .merge(routes::branding::router())
        .merge(routes::auth::router())
        .merge(routes::mfa::router())
        .merge(routes::organizations::router())
        .merge(routes::public_catalog::router())
        .merge(routes::repositories::router())
        .merge(routes::users::router())
        .merge(routes::api_tokens::router())
        .layer(CompressionLayer::new())
        .layer(axum::middleware::map_response(retry_after_when_busy))
        // Otherwise axum buffers a request body of any size before the 2 MiB branding cap ever runs.
        .layer(axum::extract::DefaultBodyLimit::max(JSON_API_BODY_LIMIT_BYTES))
        .layer(axum::middleware::from_fn_with_state(body_timeouts, limit_body_time));
    let router = Router::new()
        .merge(json_api_routes)
        // .nest_service, not .nest: the nested routers are already state-erased.
        .nest_service("/npm", artiferris_npm::router(npm_state))
        .nest_service("/v2", artiferris_docker::router(docker_state))
        .layer(TraceLayer::new_for_http())
        .layer(cors_layer(cors_allowed_origin))
        .with_state(state);
    // HSTS only when public_url is https — sending it unconditionally would lock out a plain-HTTP homelab deployment.
    with_security_headers(router, public_url.starts_with("https://"))
}

const BUSY_RETRY_AFTER_SECONDS: u64 = 5;

async fn retry_after_when_busy(mut response: axum::response::Response) -> axum::response::Response {
    use axum::http::{header, HeaderValue, StatusCode};
    if response.status() == StatusCode::SERVICE_UNAVAILABLE && !response.headers().contains_key(header::RETRY_AFTER) {
        response.headers_mut().insert(header::RETRY_AFTER, HeaderValue::from(BUSY_RETRY_AFTER_SECONDS));
    }
    response
}

/// A layer only wraps routes that exist at the point it's added — needs a second call in main() after the static-file fallback.
fn with_security_headers(router: Router, hsts_enabled: bool) -> Router {
    use axum::http::{header, HeaderValue};
    use tower_http::set_header::SetResponseHeaderLayer;

    let router = router
        .layer(SetResponseHeaderLayer::overriding(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff")))
        .layer(SetResponseHeaderLayer::overriding(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY")))
        .layer(SetResponseHeaderLayer::overriding(header::REFERRER_POLICY, HeaderValue::from_static("same-origin")))
        .layer(SetResponseHeaderLayer::overriding(header::HeaderName::from_static("permissions-policy"), HeaderValue::from_static(response_policy::PERMISSIONS_POLICY)))
        // Sign-in is a full-page redirect and passkeys need no popup, so nothing relies on window.opener.
        .layer(SetResponseHeaderLayer::overriding(header::HeaderName::from_static("cross-origin-opener-policy"), HeaderValue::from_static("same-origin")))
        .layer(axum::middleware::from_fn(response_policy::response_policy))
        .layer(SetResponseHeaderLayer::overriding(
            header::CONTENT_SECURITY_POLICY,
            // style-src needs 'unsafe-inline': Angular injects per-component <style> tags and there are no CSP nonces.
            // img-src allows https: for package README images (the sanitizer keeps only https ones); script-src is not
            // loosened.
            HeaderValue::from_static(
                "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob: https:; font-src 'self' data:; connect-src 'self'; object-src 'none'; base-uri 'self'; form-action 'self'; frame-ancestors 'none'",
            ),
        ));
    if hsts_enabled {
        router.layer(SetResponseHeaderLayer::overriding(header::STRICT_TRANSPORT_SECURITY, HeaderValue::from_static("max-age=63072000; includeSubDomains")))
    } else {
        router
    }
}

/// Reuses every adapter/use-case `AppState` already built.
fn build_npm_state(state: &AppState, public_url: &str, guard: Arc<artiferris_application::request_guard::RequestGuard>) -> artiferris_npm::NpmState {
    let remote_registry: Arc<dyn artiferris_domain::npm_remote::RemoteNpmRegistryPort> =
        Arc::new(artiferris_infrastructure::http_remote_npm_registry::HttpRemoteNpmRegistry::new());

    artiferris_npm::NpmState {
        users: state.users.clone(),
        repositories: state.repositories.clone(),
        permissions: state.permissions.clone(),
        api_tokens: state.api_tokens.clone(),
        organizations: state.organizations.clone(),
        artiferris_base_domain: state.artiferris_base_domain.clone(),
        public_scheme: if public_url.starts_with("https://") { "https".to_string() } else { "http".to_string() },
        guard,
        publish: Arc::new(artiferris_application::use_cases::npm_publish::PublishNpmPackageUseCase::new(
            state.npm_packages.clone(),
            state.storage.clone(),
            state.repositories.clone(),
            state.repository_quota_lock.clone(),
            state.events.clone(),
        )),
        metadata: Arc::new(artiferris_application::use_cases::npm_metadata::GetNpmPackageMetadataUseCase::new(
            state.npm_packages.clone(),
            state.repositories.clone(),
            remote_registry.clone(),
        )),
        download: Arc::new(artiferris_application::use_cases::npm_download::DownloadNpmTarballUseCase::new(
            state.npm_packages.clone(),
            state.storage.clone(),
            remote_registry.clone(),
            state.repositories.clone(),
        )),
        downloads: state.download_counter_buffer.clone(),
        unpublish: Arc::new(artiferris_application::use_cases::npm_unpublish::UnpublishNpmPackageUseCase::new(
            state.npm_packages.clone(),
            state.storage.clone(),
            state.events.clone(),
        )),
        deprecate: Arc::new(artiferris_application::use_cases::npm_deprecate::DeprecateNpmVersionUseCase::new(
            state.npm_packages.clone(),
            state.events.clone(),
        )),
        set_dist_tag: Arc::new(artiferris_application::use_cases::npm_dist_tags::SetDistTagUseCase::new(
            state.npm_packages.clone(),
            state.events.clone(),
        )),
        delete_dist_tag: Arc::new(artiferris_application::use_cases::npm_dist_tags::DeleteDistTagUseCase::new(state.npm_packages.clone())),
        list_dist_tags: Arc::new(artiferris_application::use_cases::npm_dist_tags::ListDistTagsUseCase::new(state.npm_packages.clone())),
        search: Arc::new(artiferris_application::use_cases::npm_search::SearchNpmPackagesUseCase::new(state.npm_packages.clone())),
        bulk_audit: Arc::new(artiferris_application::use_cases::npm_audit::BulkAuditNpmPackagesUseCase::new(state.npm_audit.clone())),
        scan_dependency_tree: state.scan_dependency_tree.clone(),
        create_api_token: state.create_api_token.clone(),
        list_api_tokens: state.list_api_tokens.clone(),
        revoke_api_token: state.revoke_api_token.clone(),
        resolve_personal_repository: Arc::new(artiferris_application::use_cases::resolve_personal_repository::ResolvePersonalRepositoryUseCase::new(
            state.users.clone(),
            state.organizations.clone(),
            state.repositories.clone(),
        )),
    }
}

/// Reuses `AppState`'s adapters; `jwt_secret`/`docker_token_realm_override`/`public_url` arrive as explicit params since `AppState` doesn't store them.
fn build_docker_state(
    state: &AppState,
    jwt_secret: &str,
    docker_token_realm_override: Option<String>,
    public_url: &str,
    guard: Arc<artiferris_application::request_guard::RequestGuard>,
) -> artiferris_docker::DockerState {
    let remote: Arc<dyn artiferris_domain::docker_remote::RemoteDockerRegistryPort> =
        Arc::new(artiferris_infrastructure::http_remote_docker_registry::HttpRemoteDockerRegistry::new());
    let token_issuer: Arc<dyn artiferris_domain::docker_registry::DockerTokenIssuerPort> =
        Arc::new(artiferris_infrastructure::jwt_docker_token_issuer::JwtDockerTokenIssuer::new(jwt_secret.to_string()));
    let list_catalog = Arc::new(artiferris_application::use_cases::docker_list::ListCatalogUseCase::new(state.docker_manifests.clone()));
    let resolve_personal_repository = Arc::new(artiferris_application::use_cases::resolve_personal_repository::ResolvePersonalRepositoryUseCase::new(
        state.users.clone(),
        state.organizations.clone(),
        state.repositories.clone(),
    ));

    let start_upload = Arc::new(artiferris_application::use_cases::docker_upload::StartBlobUploadUseCase::new(state.docker_uploads.clone()));
    let patch_upload = Arc::new(artiferris_application::use_cases::docker_upload::PatchBlobUploadUseCase::new(
        state.docker_uploads.clone(),
        state.docker_blobs.clone(),
        state.repositories.clone(),
        state.repository_quota_lock.clone(),
    ));
    let complete_upload = Arc::new(artiferris_application::use_cases::docker_upload::CompleteBlobUploadUseCase::new(state.docker_uploads.clone(), state.docker_blobs.clone()));

    artiferris_docker::DockerState {
        repositories: state.repositories.clone(),
        permissions: state.permissions.clone(),
        organizations: state.organizations.clone(),
        users: state.users.clone(),
        tokens_valid_after_cache: artiferris_docker::tokens_valid_after_cache::TokensValidAfterCache::new(
            artiferris_docker::state::TOKENS_VALID_AFTER_CACHE_TTL,
        ),
        artiferris_base_domain: state.artiferris_base_domain.clone(),
        token_issuer: token_issuer.clone(),
        token_realm_override: docker_token_realm_override,
        // Same source as the HSTS check below.
        public_scheme: if public_url.starts_with("https://") { "https".to_string() } else { "http".to_string() },
        token_service: "artiferris".to_string(),
        issue_access_token: Arc::new(artiferris_application::use_cases::docker_access_token::IssueDockerAccessTokenUseCase::new(
            state.api_tokens.clone(),
            state.users.clone(),
            state.repositories.clone(),
            state.permissions.clone(),
            token_issuer,
            resolve_personal_repository.clone(),
        )),
        login_throttle: state.login_throttle.clone(),
        guard,
        record_security_event: Arc::new(artiferris_application::use_cases::admin::RecordSecurityEventUseCase::new(state.events.clone())),
        start_upload: start_upload.clone(),
        patch_upload: patch_upload.clone(),
        complete_upload: complete_upload.clone(),
        monolithic_upload: Arc::new(artiferris_application::use_cases::docker_upload::MonolithicBlobUploadUseCase::new(start_upload, patch_upload, complete_upload, state.docker_uploads.clone())),
        put_manifest: Arc::new(artiferris_application::use_cases::docker_manifest_put::PutManifestUseCase::new(
            state.docker_manifests.clone(),
            state.repositories.clone(),
            state.events.clone(),
        )),
        get_manifest: Arc::new(artiferris_application::use_cases::docker_manifest_get::GetManifestUseCase::new(
            state.docker_manifests.clone(),
            state.repositories.clone(),
            remote.clone(),
        )),
        downloads: state.download_counter_buffer.clone(),
        cache_proxied_manifest: Arc::new(artiferris_application::use_cases::docker_manifest_cache::CacheProxiedManifestUseCase::new(
            state.docker_manifests.clone(),
        )),
        get_blob: Arc::new(artiferris_application::use_cases::docker_blob_get::GetBlobUseCase::new(
            state.docker_blobs.clone(),
            state.docker_manifests.clone(),
            state.repositories.clone(),
            remote,
        )),
        delete_manifest: Arc::new(artiferris_application::use_cases::docker_manifest_delete::DeleteManifestUseCase::new(
            state.docker_manifests.clone(),
            state.docker_blobs.clone(),
            state.events.clone(),
        )),
        list_tags: Arc::new(artiferris_application::use_cases::docker_list::ListTagsUseCase::new(state.docker_manifests.clone(), state.repositories.clone())),
        list_catalog,
        list_registry_catalog: Arc::new(artiferris_application::use_cases::docker_list::ListDockerRegistryCatalogUseCase::new(
            state.repositories.clone(),
            state.permissions.clone(),
            state.docker_manifests.clone(),
        )),
        scan_docker_image: state.scan_docker_image.clone(),
        resolve_personal_repository,
    }
}

/// Without `CORS_ALLOWED_ORIGIN`, restricted to localhost/127.0.0.1/[::1] on any port instead of reflecting any origin back.
fn cors_layer(cors_allowed_origin: Option<String>) -> CorsLayer {
    match cors_allowed_origin {
        Some(origin) => CorsLayer::new()
            .allow_origin(origin.parse::<axum::http::HeaderValue>().expect("CORS_ALLOWED_ORIGIN must be a valid origin"))
            .allow_methods(tower_http::cors::Any)
            .allow_headers(tower_http::cors::Any),
        None => CorsLayer::new()
            .allow_origin(AllowOrigin::predicate(|origin, _| is_local_dev_origin(origin)))
            .allow_methods(tower_http::cors::Any)
            .allow_headers(tower_http::cors::Any),
    }
}

fn is_local_dev_origin(origin: &axum::http::HeaderValue) -> bool {
    let Ok(origin) = origin.to_str() else { return false };
    // No path in an Origin header, so exact-prefix-then-digits can't be fooled by "localhost.evil.com".
    for host in ["http://localhost", "http://127.0.0.1", "http://[::1]"] {
        if origin == host {
            return true;
        }
        if let Some(port) = origin.strip_prefix(host).and_then(|rest| rest.strip_prefix(':'))
            && !port.is_empty()
            && port.chars().all(|c| c.is_ascii_digit())
        {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum_extra::headers::HeaderMapExt;
    use tower::ServiceExt;
    use uuid::Uuid;

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

    #[test]
    fn the_audit_backfill_needs_a_pool_of_three_unless_forced() {
        assert!(!audit_backfill_may_run(1, false));
        assert!(!audit_backfill_may_run(2, false));
        assert!(audit_backfill_may_run(3, false));
        assert!(audit_backfill_may_run(10, false));
        assert!(audit_backfill_may_run(1, true));
    }

    #[sqlx::test]
    async fn healthz_returns_ok(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let app = build_router(state);

        let response = app.oneshot(Request::builder().uri("/healthz").body(Body::empty()).unwrap()).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[sqlx::test]
    async fn readyz_is_ok_while_the_database_answers(pool: sqlx::PgPool) {
        let app = build_router(AppState::build(pool, &test_config()));

        let response = app.oneshot(Request::builder().uri("/readyz").body(Body::empty()).unwrap()).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[sqlx::test]
    async fn readyz_fails_as_soon_as_a_stop_is_requested_and_healthz_stays_ok(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let app = build_router(state.clone());
        state.shutting_down.store(true, std::sync::atomic::Ordering::Relaxed);

        let readyz = app.clone().oneshot(Request::builder().uri("/readyz").body(Body::empty()).unwrap()).await.unwrap();
        let healthz = app.oneshot(Request::builder().uri("/healthz").body(Body::empty()).unwrap()).await.unwrap();

        assert_eq!(readyz.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(healthz.status(), StatusCode::OK, "a pod that is stopping is not killed for it");
    }

    #[test]
    fn the_shutdown_settings_default_when_unset_and_reject_a_typo() {
        assert_eq!(parse_seconds("X", None, 25).unwrap(), std::time::Duration::from_secs(25));
        assert_eq!(parse_seconds("X", Some("  "), 25).unwrap(), std::time::Duration::from_secs(25));
        assert_eq!(parse_seconds("X", Some("0"), 25).unwrap(), std::time::Duration::ZERO);
        assert_eq!(parse_seconds("X", Some("120"), 25).unwrap(), std::time::Duration::from_secs(120));
        assert!(parse_seconds("X", Some("10s"), 25).is_err());
        assert!(parse_seconds("X", Some("-1"), 25).is_err());
    }

    #[sqlx::test]
    async fn readyz_fails_and_healthz_stays_ok_when_the_database_is_gone(pool: sqlx::PgPool) {
        let app = build_router(AppState::build(pool.clone(), &test_config()));
        pool.close().await;

        let readyz = app.clone().oneshot(Request::builder().uri("/readyz").body(Body::empty()).unwrap()).await.unwrap();
        let healthz = app.oneshot(Request::builder().uri("/healthz").body(Body::empty()).unwrap()).await.unwrap();

        assert_eq!(readyz.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(healthz.status(), StatusCode::OK);
    }

    #[sqlx::test]
    async fn readyz_stays_ok_and_answers_at_once_while_every_pool_connection_is_busy(pool: sqlx::PgPool) {
        let small = sqlx::postgres::PgPoolOptions::new().max_connections(1).acquire_timeout(std::time::Duration::from_secs(30)).connect_with((*pool.connect_options()).clone()).await.unwrap();
        let app = build_router(AppState::build(small.clone(), &test_config()));
        let probe = || Request::builder().uri("/readyz").body(Body::empty()).unwrap();
        assert_eq!(app.clone().oneshot(probe()).await.unwrap().status(), StatusCode::OK);
        let _busy = small.acquire().await.unwrap();

        let started = std::time::Instant::now();
        let response = app.oneshot(probe()).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }

    fn healthz_from(origin: &str) -> Request<Body> {
        Request::builder().uri("/healthz").header("origin", origin).body(Body::empty()).unwrap()
    }

    fn router_with(pool: sqlx::PgPool, config: &Config, cors_allowed_origin: Option<String>) -> Router {
        build_router_with_cors(AppState::build(pool, config), cors_allowed_origin, config.jwt_secret.clone(), config.docker_token_realm_override.clone(), config.public_url.clone())
    }

    #[test]
    fn is_local_dev_origin_accepts_localhost_127_0_0_1_and_ipv6_loopback_on_any_port() {
        for origin in ["http://localhost", "http://localhost:4200", "http://127.0.0.1", "http://127.0.0.1:8080", "http://[::1]", "http://[::1]:4200"] {
            assert!(super::is_local_dev_origin(&origin.parse().unwrap()), "expected {origin} to be accepted");
        }
    }

    #[test]
    fn is_local_dev_origin_rejects_lookalikes_and_arbitrary_origins() {
        for origin in ["https://evil.example", "http://localhost.evil.example", "http://localhost:1234.evil.example", "http://notlocalhost:4200", "http://localhost:"] {
            assert!(!super::is_local_dev_origin(&origin.parse().unwrap()), "expected {origin} to be rejected");
        }
    }

    #[sqlx::test]
    async fn a_localhost_dev_server_is_allowed_when_no_origin_is_configured(pool: sqlx::PgPool) {
        let app = router_with(pool, &test_config(), None);

        let response = app.oneshot(healthz_from("http://localhost:4200")).await.unwrap();

        // Reflects the exact origin, not "*" — browsers reject "*" for credentialed requests.
        assert_eq!(response.headers().get("access-control-allow-origin").unwrap(), "http://localhost:4200");
    }

    #[sqlx::test]
    async fn a_127_0_0_1_dev_server_is_allowed_when_no_origin_is_configured(pool: sqlx::PgPool) {
        let app = router_with(pool, &test_config(), None);

        let response = app.oneshot(healthz_from("http://127.0.0.1:4200")).await.unwrap();

        assert_eq!(response.headers().get("access-control-allow-origin").unwrap(), "http://127.0.0.1:4200");
    }

    #[sqlx::test]
    async fn an_arbitrary_internet_origin_is_rejected_when_no_origin_is_configured(pool: sqlx::PgPool) {
        let app = router_with(pool, &test_config(), None);

        let response = app.oneshot(healthz_from("https://evil.example")).await.unwrap();

        assert!(response.headers().get("access-control-allow-origin").is_none(), "an unconfigured deployment must not reflect an arbitrary origin");
    }

    #[sqlx::test]
    async fn a_lookalike_hostname_is_not_confused_with_localhost(pool: sqlx::PgPool) {
        let app = router_with(pool, &test_config(), None);

        let response = app.oneshot(healthz_from("http://localhost.evil.example")).await.unwrap();

        assert!(response.headers().get("access-control-allow-origin").is_none());
    }

    #[sqlx::test]
    async fn a_configured_origin_replaces_the_permissive_wildcard(pool: sqlx::PgPool) {
        let app = router_with(pool, &test_config(), Some("https://artiferris.example".to_string()));

        let response = app.oneshot(healthz_from("https://artiferris.example")).await.unwrap();

        assert_eq!(response.headers().get("access-control-allow-origin").unwrap(), "https://artiferris.example");
    }

    #[sqlx::test]
    async fn a_configured_origin_is_not_echoed_back_to_other_origins(pool: sqlx::PgPool) {
        let app = router_with(pool, &test_config(), Some("https://artiferris.example".to_string()));

        let response = app.oneshot(healthz_from("https://evil.example")).await.unwrap();

        assert_eq!(response.headers().get("access-control-allow-origin").unwrap(), "https://artiferris.example");
    }

    #[sqlx::test]
    async fn every_response_carries_the_baseline_security_headers(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let app = build_router(state);

        let response = app.oneshot(Request::builder().uri("/healthz").body(Body::empty()).unwrap()).await.unwrap();

        let headers = response.headers();
        assert_eq!(headers.get("x-content-type-options").unwrap(), "nosniff");
        assert_eq!(headers.get("x-frame-options").unwrap(), "DENY");
        assert_eq!(headers.get("referrer-policy").unwrap(), "same-origin");
        let csp = headers.get("content-security-policy").unwrap().to_str().unwrap();
        assert!(csp.contains("default-src 'self'"));
        assert!(csp.contains("frame-ancestors 'none'"));
        let img_src = csp.split(';').map(|directive| directive.trim()).find(|directive| directive.starts_with("img-src")).expect("CSP must declare an img-src directive");
        assert!(img_src.split_whitespace().any(|source| source == "https:"), "README images are https-only, so https: must be allowed: {img_src}");
        assert!(!img_src.contains("http:") && !img_src.contains('*'), "plain http and wildcards stay out: {img_src}");
        // The frontend keeps its session JWT in sessionStorage, not an HttpOnly cookie, an accepted risk that holds
        // only while script injection is structurally blocked: a strict script-src without unsafe-inline or
        // unsafe-eval. If this assertion ever changes, revisit that acceptance first.
        //
        // style-src legitimately carries 'unsafe-inline' (Angular's per-component styles), so the check must isolate
        // the script-src directive instead of scanning the whole header.
        let script_src = csp
            .split(';')
            .map(|directive| directive.trim())
            .find(|directive| directive.starts_with("script-src"))
            .expect("CSP must declare a script-src directive");
        assert_eq!(script_src, "script-src 'self'");
        assert!(!script_src.contains("unsafe-inline"));
        assert!(!script_src.contains("unsafe-eval"));
    }

    #[sqlx::test]
    async fn every_response_also_carries_the_permissions_and_opener_policies(pool: sqlx::PgPool) {
        let app = build_router(AppState::build(pool, &test_config()));

        for uri in ["/healthz", "/api/version", "/npm/", "/v2/"] {
            let response = app.clone().oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap()).await.unwrap();
            let headers = response.headers();
            assert_eq!(headers.get("cross-origin-opener-policy").unwrap(), "same-origin", "{uri}");
            let permissions = headers.get("permissions-policy").unwrap().to_str().unwrap();
            for feature in ["camera=()", "microphone=()", "geolocation=()", "payment=()", "usb=()"] {
                assert!(permissions.contains(feature), "{uri}: {permissions}");
            }
        }
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn json_api_responses_are_not_stored_and_vary_on_the_caller(pool: sqlx::PgPool) {
        use artiferris_domain::package_repository::{RepositoryFormat, RepositoryType};

        let state = AppState::build(pool, &test_config());
        let public_org = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let admin_id = state.create_user.execute(public_org, "admin", "sup3r-s3cret!", true).await.unwrap();
        let repo_id = state.create_repository.execute(public_org, "open", RepositoryFormat::Npm, RepositoryType::Hosted, None, None, None, admin_id).await.unwrap();
        state.set_repository_visibility.execute(repo_id, true, admin_id).await.unwrap();
        let token = state.authenticate_user.execute("admin", "sup3r-s3cret!").await.unwrap();
        let app = build_router(state);
        let get = |uri: String, token: Option<&str>| {
            let mut request = Request::builder().uri(uri);
            if let Some(token) = token {
                request = request.header("authorization", format!("Bearer {token}"));
            }
            app.clone().oneshot(request.body(Body::empty()).unwrap())
        };

        for (uri, token) in [
            (format!("/api/repositories/{repo_id}"), None),
            (format!("/api/repositories/{repo_id}"), Some(token.as_str())),
            ("/api/repositories".to_string(), Some(token.as_str())),
            ("/api/version".to_string(), None),
            ("/api/no-such-route".to_string(), None),
        ] {
            let response = get(uri.clone(), token).await.unwrap();
            let headers = response.headers();
            assert_eq!(headers.get("cache-control").unwrap(), "no-store", "{uri}");
            assert_eq!(headers.get_all("vary").iter().filter(|v| v.to_str().unwrap().contains("Authorization")).count(), 1, "{uri}: {:?}", headers.get_all("vary"));
            assert_eq!(headers.get("cross-origin-resource-policy").unwrap(), "same-origin", "{uri}");
        }
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn the_public_catalog_and_branding_keep_the_cache_headers_their_handlers_chose(pool: sqlx::PgPool) {
        let app = build_router(AppState::build(pool, &test_config()));

        let catalog = app.clone().oneshot(Request::builder().uri("/api/public/catalogs").body(Body::empty()).unwrap()).await.unwrap();
        let logo = app.oneshot(Request::builder().uri("/api/branding/logo").body(Body::empty()).unwrap()).await.unwrap();

        assert_eq!(catalog.headers().get_all("cache-control").iter().count(), 1);
        assert_eq!(catalog.headers().get("cache-control").unwrap(), "no-store");
        assert_eq!(logo.status(), StatusCode::OK);
        assert_eq!(logo.headers().get("cache-control").unwrap(), "no-cache", "the branding handler's own choice stays");
        assert!(logo.headers().get("cross-origin-resource-policy").is_none(), "link previews on other sites embed the logo");
    }

    #[sqlx::test]
    async fn the_registries_are_not_given_a_cross_origin_resource_policy(pool: sqlx::PgPool) {
        let app = build_router(AppState::build(pool, &test_config()));

        for uri in ["/npm/", "/v2/"] {
            let response = app.clone().oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap()).await.unwrap();
            assert!(response.headers().get("cross-origin-resource-policy").is_none(), "{uri}: npm and docker clients, proxies and CDNs fetch these");
            assert!(response.headers().get("vary").is_none_or(|v| !v.to_str().unwrap().is_empty()));
        }
    }

    #[sqlx::test]
    async fn the_app_pages_get_the_same_origin_policies_and_keep_their_own_caching(pool: sqlx::PgPool) {
        let static_dir = std::env::temp_dir().join(format!("artiferris-headers-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&static_dir).unwrap();
        std::fs::write(static_dir.join("index.html"), "<!doctype html><html><head><title>x</title></head><body><app-root></app-root></body></html>").unwrap();
        std::fs::write(static_dir.join("main.js"), "console.log(1)").unwrap();
        let state = AppState::build(pool, &test_config());
        let seo = seo::SeoState::new(state.clone(), "<!doctype html><html><head><title>x</title></head><body></body></html>".to_string());
        let app = with_app_fallback(build_router(state), static_dir.to_str().unwrap(), seo, false);

        for uri in ["/", "/explorer", "/main.js"] {
            let response = app.clone().oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap()).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK, "{uri}");
            let headers = response.headers();
            assert_eq!(headers.get("cross-origin-resource-policy").unwrap(), "same-origin", "{uri}");
            assert_eq!(headers.get("cross-origin-opener-policy").unwrap(), "same-origin", "{uri}");
            assert!(headers.get("permissions-policy").is_some(), "{uri}");
            assert!(headers.get("vary").is_none_or(|v| !v.to_str().unwrap().contains("Authorization")), "{uri}: only /api varies on the caller");
        }
        std::fs::remove_dir_all(&static_dir).unwrap();
    }

    fn stalled_json_body() -> Body {
        use futures::StreamExt;
        Body::from_stream(futures::stream::iter([Ok::<_, std::convert::Infallible>(axum::body::Bytes::from_static(b"{\"username\":"))]).chain(futures::stream::pending()))
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_json_body_that_stalls_gets_a_408_instead_of_holding_the_request(pool: sqlx::PgPool) {
        let config = test_config();
        let timeouts = BodyTimeouts { idle: std::time::Duration::from_millis(100), total: std::time::Duration::from_secs(30) };
        let app = build_router_with_body_timeouts(AppState::build(pool, &config), None, config.jwt_secret.clone(), None, config.public_url.clone(), timeouts);
        let request = Request::builder().method("POST").uri("/api/auth/login").header("content-type", "application/json").body(stalled_json_body()).unwrap();

        let response = tokio::time::timeout(std::time::Duration::from_secs(5), app.oneshot(request)).await.expect("the request was still waiting for its body").unwrap();

        assert_eq!(response.status(), StatusCode::REQUEST_TIMEOUT);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_json_body_that_trickles_past_the_total_limit_gets_a_408(pool: sqlx::PgPool) {
        let config = test_config();
        let timeouts = BodyTimeouts { idle: std::time::Duration::from_secs(5), total: std::time::Duration::from_millis(300) };
        let app = build_router_with_body_timeouts(AppState::build(pool, &config), None, config.jwt_secret.clone(), None, config.public_url.clone(), timeouts);
        let trickle = futures::stream::unfold(0u8, |n| async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            Some((Ok::<_, std::convert::Infallible>(axum::body::Bytes::from_static(b" ")), n + 1))
        });
        let request = Request::builder().method("POST").uri("/api/auth/login").header("content-type", "application/json").body(Body::from_stream(trickle)).unwrap();

        let response = tokio::time::timeout(std::time::Duration::from_secs(5), app.oneshot(request)).await.expect("the request was still waiting for its body").unwrap();

        assert_eq!(response.status(), StatusCode::REQUEST_TIMEOUT);
    }

    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_json_body_that_arrives_in_time_is_handled_normally(pool: sqlx::PgPool) {
        let config = test_config();
        let timeouts = BodyTimeouts { idle: std::time::Duration::from_millis(100), total: std::time::Duration::from_secs(30) };
        let app = build_router_with_body_timeouts(AppState::build(pool, &config), None, config.jwt_secret.clone(), None, config.public_url.clone(), timeouts);
        let request = Request::builder().method("POST").uri("/api/auth/login").header("content-type", "application/json").body(Body::from(r#"{"username":"nobody","password":"wrong-password"}"#)).unwrap();

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[sqlx::test]
    async fn version_endpoint_reports_the_crate_version(pool: sqlx::PgPool) {
        let state = AppState::build(pool, &test_config());
        let app = build_router(state);

        let response = app.oneshot(Request::builder().uri("/api/version").body(Body::empty()).unwrap()).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["version"], env!("CARGO_PKG_VERSION"));
    }

    #[sqlx::test]
    async fn hsts_is_absent_when_public_url_is_plain_http(pool: sqlx::PgPool) {
        let config = test_config();
        let app = build_router_with_cors(AppState::build(pool, &config), None, config.jwt_secret.clone(), config.docker_token_realm_override.clone(), "http://artiferris.example".to_string());

        let response = app.oneshot(Request::builder().uri("/healthz").body(Body::empty()).unwrap()).await.unwrap();

        assert!(response.headers().get("strict-transport-security").is_none(), "sending HSTS for a plain-HTTP deployment risks locking operators out over HTTP");
    }

    #[sqlx::test]
    async fn hsts_is_present_when_public_url_is_https(pool: sqlx::PgPool) {
        let config = test_config();
        let app = build_router_with_cors(AppState::build(pool, &config), None, config.jwt_secret.clone(), config.docker_token_realm_override.clone(), "https://artiferris.example".to_string());

        let response = app.oneshot(Request::builder().uri("/healthz").body(Body::empty()).unwrap()).await.unwrap();

        assert!(response.headers().get("strict-transport-security").unwrap().to_str().unwrap().contains("max-age="));
    }

    // A docker route's Location header must resolve through /v2, not a bare / — exercises main()'s nested wiring.
    #[sqlx::test(migrations = "../artiferris-infrastructure/migrations")]
    async fn a_docker_blob_uploads_location_header_resolves_back_into_the_nested_v2_router(pool: sqlx::PgPool) {
        use artiferris_domain::package_repository::{RepositoryFormat, RepositoryType};

        let config = test_config();
        let state = AppState::build(pool, &config);
        let admin_id = state.create_user.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "admin", "sup3r-s3cret!", true).await.unwrap();
        state.create_repository.execute(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(), "docker-verify-regression", RepositoryFormat::Docker, RepositoryType::Hosted, None, None, None, admin_id).await.unwrap();
        let (_token_id, plaintext_token) = state.create_api_token.execute(admin_id, "docker-test").await.unwrap();
        let app = build_router(state);

        let mut basic_auth_headers = axum::http::HeaderMap::new();
        basic_auth_headers.typed_insert(axum_extra::headers::Authorization::basic("anything", &plaintext_token));
        let basic_auth = basic_auth_headers.get(axum::http::header::AUTHORIZATION).unwrap().clone();
        let token_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v2/token?scope=repository:docker-verify-regression/myimage:pull,push")
                    .header(axum::http::header::AUTHORIZATION, basic_auth)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(token_response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(token_response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let bearer = json["token"].as_str().unwrap().to_string();

        let start_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v2/docker-verify-regression/myimage/blobs/uploads/")
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {bearer}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(start_response.status(), StatusCode::ACCEPTED);
        let location = start_response.headers().get(axum::http::header::LOCATION).unwrap().to_str().unwrap().to_string();
        assert!(location.starts_with("/v2/"), "Location header {location:?} must be reachable through this router's own /v2 mount");

        let blob_bytes = b"regression-test-blob".to_vec();
        let digest = artiferris_domain::docker_registry::Digest::of(&blob_bytes);
        let complete_response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("{location}?digest={}", digest.as_str()))
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {bearer}"))
                    .body(Body::from(blob_bytes))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(complete_response.status(), StatusCode::CREATED);
    }
}
