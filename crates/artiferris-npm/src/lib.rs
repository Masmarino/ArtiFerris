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

pub fn router(state: NpmState) -> axum::Router {
    axum::Router::new()
        .merge(routes::metadata::router())
        .merge(routes::publish::router())
        .merge(routes::unpublish::router())
        .merge(routes::dist_tags::router())
        .merge(routes::search::router())
        .merge(routes::advisories::router())
        // Real `npm audit` sends its bulk advisory request gzip-compressed
        // unconditionally — without this, axum rejects it with a 400. Handlers read their bodies
        // themselves, after authorizing, and cap them at their own limit (`body::read_json`), which
        // applies to the decompressed bytes.
        .layer(tower_http::decompression::RequestDecompressionLayer::new())
        .with_state(state)
}
