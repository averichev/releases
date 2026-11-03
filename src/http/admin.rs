use axum::{
    Router,
    extract::Request,
    middleware::{self, Next},
    response::Response,
    routing::any,
};

use super::{AppState, error::ApiError, not_found};

pub fn router() -> Router<AppState> {
    Router::new()
        // Explicit roots avoid nest's differing treatment of trailing slashes.
        .route("/admin", any(not_found))
        .route("/admin/", any(not_found))
        .route("/admin/{*path}", any(not_found))
        .layer(middleware::from_fn(disabled_until_authentication))
}

// Replace this deny-all gate with real authentication before adding admin routes.
// Keeping the gate at router scope also covers unknown paths and HTTP methods.
async fn disabled_until_authentication(
    _request: Request,
    _next: Next,
) -> Result<Response, ApiError> {
    Err(ApiError::AdministrationUnavailable)
}
