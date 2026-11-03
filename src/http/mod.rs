mod admin;
mod assets;
mod error;
mod health;

use axum::{
    Router,
    http::{HeaderValue, header},
    routing::{any, get},
};
use sqlx::SqlitePool;
use tower_http::{
    compression::CompressionLayer,
    set_header::SetResponseHeaderLayer,
    trace::{DefaultOnResponse, TraceLayer},
};
use tracing::Level;

use error::ApiError;

#[derive(Clone)]
pub struct AppState {
    pub database: SqlitePool,
}

pub fn router(database: SqlitePool) -> Router {
    let public = Router::new().route("/health", get(health::get));
    let v1 = public.merge(admin::router()).fallback(not_found);
    let api = Router::new().nest("/v1", v1).fallback(not_found).layer(
        SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        ),
    );

    Router::new()
        .route("/api", any(not_found))
        .nest("/api", api)
        .fallback(assets::serve)
        .method_not_allowed_fallback(|| async { ApiError::MethodNotAllowed })
        .with_state(AppState { database })
        .layer(CompressionLayer::new())
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff")))
        .layer(TraceLayer::new_for_http()
            // Avoid logging raw query strings, credentials and future signed URLs.
            .make_span_with(|request: &axum::http::Request<axum::body::Body>| {
                tracing::info_span!("http_request", method = %request.method(), path = %request.uri().path())
            })
            .on_response(DefaultOnResponse::new().level(Level::INFO)))
}

async fn not_found() -> ApiError {
    ApiError::NotFound
}
