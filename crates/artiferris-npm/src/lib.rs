pub mod auth;
pub mod authz;
pub mod body;
pub mod errors;
pub mod organization_resolution;
pub mod routes;
pub mod state;

#[cfg(test)]
pub mod test_support;

pub use state::NpmState;
mod anonymous_limit;

pub fn router(state: NpmState) -> axum::Router {
    axum::Router::new()
        .merge(routes::metadata::router())
        .merge(routes::publish::router())
        .merge(routes::unpublish::router())
        .merge(routes::dist_tags::router())
        .merge(routes::search::router())
        .merge(routes::advisories::router())
        // Real `npm audit` sends its bulk request gzip-compressed, which axum would reject with a 400. Handlers read
        // their own bodies after authorizing and cap them at their own limit (`body::read_json`), applied to the
        // decompressed bytes.
        .layer(tower_http::decompression::RequestDecompressionLayer::new())
        .layer(axum::middleware::from_fn_with_state(state.clone(), anonymous_limit::limit_anonymous_reads))
        .with_state(state)
}
