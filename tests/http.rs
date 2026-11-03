use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode},
    response::Response,
};
use http_body_util::BodyExt;
use serde_json::Value;
use sqlx::SqlitePool;
use tempfile::{TempDir, tempdir};
use tower::ServiceExt;
use veruna_releases::{config::AppConfig, http, initialize};

async fn setup() -> (TempDir, SqlitePool, Router) {
    let directory = tempdir().unwrap();
    let mut config = AppConfig::default();
    config.database.path = directory.path().join("database.sqlite");
    config.storage.directory = directory.path().join("artifacts");
    let pool = initialize(&config).await.unwrap();
    let router = http::router(pool.clone());
    (directory, pool, router)
}

async fn request(router: &Router, method: Method, path: &str) -> Response {
    router
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn body(response: Response) -> Vec<u8> {
    response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec()
}

#[tokio::test]
async fn health_checks_database_and_returns_safe_errors() {
    let (_directory, pool, router) = setup().await;
    let response = request(&router, Method::GET, "/api/v1/health").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "application/json");
    assert_eq!(response.headers()["cache-control"], "no-store");
    let json: Value = serde_json::from_slice(&body(response).await).unwrap();
    assert_eq!(
        json,
        serde_json::json!({"status":"ok", "service":"veruna-releases", "version": env!("CARGO_PKG_VERSION")})
    );
    pool.close().await;
    let response = request(&router, Method::GET, "/api/v1/health").await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let json: Value = serde_json::from_slice(&body(response).await).unwrap();
    assert_eq!(
        json,
        serde_json::json!({"error":{"code":"database_unavailable","message":"Service is not ready"}})
    );
}

#[tokio::test]
async fn api_and_asset_errors_never_fall_back_to_html() {
    let (_directory, pool, router) = setup().await;
    for path in [
        "/api",
        "/api/",
        "/api/v1",
        "/api/v1/",
        "/api/v1/missing",
        "/api/v2/health",
        "/assets/missing.js",
        "/admin/missing.js",
        "/artifacts/private.zip",
        "/unknown",
        "/favicon-missing.svg",
    ] {
        let response = request(&router, Method::GET, path).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
        assert_eq!(
            response.headers()["content-type"],
            "application/json",
            "{path}"
        );
        let json: Value = serde_json::from_slice(&body(response).await).unwrap();
        assert_eq!(json["error"]["code"], "not_found");
    }
    let response = request(&router, Method::POST, "/api/v1/health").await;
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(response.headers()["content-type"], "application/json");
    pool.close().await;
}

#[tokio::test]
async fn administration_denies_all_paths_and_methods_until_auth_exists() {
    let (_directory, pool, router) = setup().await;
    for path in [
        "/api/v1/admin",
        "/api/v1/admin/",
        "/api/v1/admin/applications",
        "/api/v1/admin/nested/missing",
    ] {
        for method in [
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::OPTIONS,
        ] {
            let response = request(&router, method, path).await;
            assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE, "{path}");
            let json: Value = serde_json::from_slice(&body(response).await).unwrap();
            assert_eq!(json["error"]["code"], "administration_unavailable");
        }
    }
    pool.close().await;
}

#[tokio::test]
async fn embedded_ui_supports_nested_documents_assets_and_head() {
    let (_directory, pool, router) = setup().await;
    let index = body(request(&router, Method::GET, "/").await).await;
    for path in [
        "/",
        "/admin",
        "/admin/",
        "/admin/releases",
        "/admin/applications/example/releases",
    ] {
        let response = request(&router, Method::GET, path).await;
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        assert_eq!(
            response.headers()["content-type"],
            "text/html; charset=utf-8"
        );
        assert_eq!(response.headers()["cache-control"], "no-cache");
        assert_eq!(response.headers()["x-content-type-options"], "nosniff");
        assert_eq!(body(response).await, index);
    }
    let html = String::from_utf8(index.clone()).unwrap();
    let script = html
        .split("src=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    assert!(script.starts_with("/assets/"));
    let response = request(&router, Method::GET, script).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response.headers()["content-type"]
            .to_str()
            .unwrap()
            .contains("javascript")
    );
    assert_eq!(
        response.headers()["cache-control"],
        "public, max-age=31536000, immutable"
    );
    let script_body = body(response).await;
    assert!(!script_body.is_empty());
    for (path, length) in [
        ("/admin/releases", index.len()),
        (script, script_body.len()),
    ] {
        let response = request(&router, Method::HEAD, path).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()["content-length"].to_str().unwrap(),
            length.to_string()
        );
        assert!(body(response).await.is_empty());
    }
    pool.close().await;
}
