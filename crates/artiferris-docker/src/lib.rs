pub mod auth;
pub mod authz;
pub mod errors;
pub mod organization_resolution;
pub mod routes;
pub mod state;
pub mod tokens_valid_after_cache;

#[cfg(test)]
pub mod route_test_support;

pub use state::DockerState;
mod anonymous_limit;

pub fn router(state: DockerState) -> axum::Router {
    axum::Router::new()
        .merge(routes::handshake::router())
        .merge(routes::dispatch::router())
        .merge(routes::catalog::router())
        .layer(axum::middleware::from_fn_with_state(state.clone(), anonymous_limit::limit_anonymous_reads))
        .with_state(state)
}
