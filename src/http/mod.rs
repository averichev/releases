mod admin;
mod applications;
mod artifacts;
mod assets;
mod auth;
mod delivery;
mod distribution;
mod error;
mod health;
mod icons;
mod origin;
mod publication;
mod releases;
mod standalone;
mod tauri_integration;
mod upload_body;

use axum::{
    Router,
    http::{HeaderValue, Method, header},
    routing::{any, get},
};
use sqlx::SqlitePool;
use tower_http::{
    compression::{
        CompressionLayer,
        predicate::{DefaultPredicate, NotForContentType, Predicate},
    },
    cors::{Any, CorsLayer},
    set_header::SetResponseHeaderLayer,
    trace::{DefaultOnResponse, TraceLayer},
};
use tracing::Level;

use error::ApiError;

#[derive(Clone)]
pub struct AppState {
    pub server_bind: std::net::SocketAddr,
    pub distribution: crate::services::distribution::PublicDistribution,
    pub publishing: crate::services::publishing::ReleasePublishing,
    pub manifests: crate::services::manifest::ManifestGeneration,
    pub tauri_integration: crate::services::tauri_integration::TauriIntegration,
    pub updater: crate::services::updater::TauriUpdater,
    pub standalone_updater: crate::services::standalone::StandaloneUpdater,
    pub storage_config: crate::config::StorageConfig,
    pub releases: crate::services::release::ReleaseManagement,
    pub artifacts: crate::services::artifact::ArtifactManagement,
    pub database: SqlitePool,
    pub auth_config: crate::config::AuthConfig,
    pub authentication: crate::services::auth::Authentication,
    pub sessions: crate::services::sessions::Sessions,
    pub users: crate::services::users::UserManagement,
    pub applications: crate::services::applications::ApplicationCatalog,
}

pub fn router(state: AppState) -> Router {
    let resolve_origin = axum::middleware::from_fn_with_state(state.clone(), origin::resolve);
    let api_cache = SetResponseHeaderLayer::overriding(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store"),
    );
    let public_distribution = Router::new()
        .nest(
            "/api/v1",
            distribution::api_router()
                .merge(standalone::api_router())
                .layer(api_cache.clone()),
        )
        .route("/{slug}/manifest.json", get(publication::public_manifest))
        .method_not_allowed_fallback(|| async { ApiError::MethodNotAllowed })
        // Keep CORS outside origin resolution so its errors are readable too.
        .layer(resolve_origin.clone())
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods([Method::GET, Method::HEAD])
                .allow_headers([header::IF_NONE_MATCH])
                .expose_headers([header::ETAG]),
        );
    let public = Router::new().route("/health", get(health::get));
    let v1 = public
        .nest("/auth", auth::router())
        .merge(admin::router(state.clone()))
        .fallback(not_found);
    let api = Router::new()
        .nest("/v1", v1)
        .fallback(not_found)
        .layer(api_cache);

    Router::new()
        .merge(distribution::documents_router())
        .route("/admin", get(assets::serve))
        .route("/admin/", get(assets::serve))
        .route("/admin/{*path}", get(assets::panel))
        .route("/favicon.svg", get(assets::serve))
        .route("/favicon.ico", get(assets::serve))
        .route("/apple-touch-icon.png", get(assets::serve))
        .route("/index.html", get(assets::serve))
        .route("/api", any(not_found))
        .nest("/api", api)
        .fallback(assets::serve)
        .method_not_allowed_fallback(|| async { ApiError::MethodNotAllowed })
        .layer(axum::middleware::from_fn(auth::same_origin))
        .layer(resolve_origin)
        .merge(public_distribution)
        .layer(axum::extract::DefaultBodyLimit::max(8192))
        .with_state(state)
        .layer(SetResponseHeaderLayer::overriding(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer")))
        .layer(SetResponseHeaderLayer::if_not_present(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static("default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self'; object-src 'none'; base-uri 'self'; frame-ancestors 'none'")))
        .layer(CompressionLayer::new().compress_when(DefaultPredicate::new().and(NotForContentType::const_new("application/octet-stream")).and(|_: axum::http::StatusCode, _: axum::http::Version, headers: &axum::http::HeaderMap, _: &axum::http::Extensions| !headers.contains_key(header::CONTENT_DISPOSITION))))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff")))
        .layer(TraceLayer::new_for_http()
            // Avoid logging raw query strings, credentials and future signed URLs.
            .make_span_with(|request: &axum::http::Request<axum::body::Body>| {
                tracing::info_span!("http_request", method = %request.method(), route = request.extensions().get::<axum::extract::MatchedPath>().map_or("<unmatched>", |p| p.as_str()))
            })
            .on_response(DefaultOnResponse::new().level(Level::INFO)))
}

async fn not_found() -> ApiError {
    ApiError::NotFound
}
