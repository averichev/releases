use axum::{
    body::Body,
    http::{Method, Uri, header},
    response::{IntoResponse, Response},
};
use rust_embed::RustEmbed;

use super::error::ApiError;

#[derive(RustEmbed)]
#[folder = "frontend/dist/"]
struct Assets;

pub async fn serve(method: Method, uri: Uri) -> Response {
    if method != Method::GET && method != Method::HEAD {
        return ApiError::MethodNotAllowed.into_response();
    }
    let path = uri.path().trim_start_matches('/');
    if !path.is_empty()
        && let Some(asset) = Assets::get(path)
    {
        let mime = mime_guess::from_path(path)
            .first_or_octet_stream()
            .to_string();
        let cache = if path.starts_with("assets/") {
            "public, max-age=31536000, immutable"
        } else {
            "no-cache"
        };
        return file_response(asset, &mime, cache, &method);
    }
    // SPA fallback is scoped to document routes. Missing assets and unknown API
    // namespaces must never succeed with an HTML body.
    let document_route = path.is_empty()
        || path == "admin"
        || path == "admin/"
        || (path.starts_with("admin/") && !path.contains('.'));
    if document_route {
        return match Assets::get("index.html") {
            Some(asset) => file_response(asset, "text/html; charset=utf-8", "no-cache", &method),
            None => {
                tracing::error!("Embedded index.html is missing");
                ApiError::Internal.into_response()
            }
        };
    }
    ApiError::NotFound.into_response()
}

fn file_response(
    asset: rust_embed::EmbeddedFile,
    mime: &str,
    cache: &str,
    method: &Method,
) -> Response {
    let length = asset.data.len().to_string();
    let body = if *method == Method::HEAD {
        Body::empty()
    } else {
        Body::from(asset.data.into_owned())
    };
    (
        [
            (header::CONTENT_TYPE, mime),
            (header::CACHE_CONTROL, cache),
            (header::CONTENT_LENGTH, &length),
        ],
        body,
    )
        .into_response()
}
