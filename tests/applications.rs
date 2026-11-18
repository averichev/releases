use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
    response::Response,
};
use http_body_util::BodyExt;
use image::{DynamicImage, ImageFormat};
use serde_json::{Value, json};
use std::io::Cursor;
use tempfile::{TempDir, tempdir};
use tower::ServiceExt;
use veruna_releases::{
    application_state,
    config::AppConfig,
    domain::{
        application::{ApplicationChanges, NewApplication, RESERVED_SLUGS, Status, Visibility},
        user::Role,
    },
    http::{self, AppState},
    infrastructure::{
        icons::{IconStorage, MAX_ICON_BYTES},
        secrets,
    },
    initialize,
    services::{error::ServiceError, sessions::Identity},
};
const BASE: &str = "/api/v1/admin/applications";
struct TestApp {
    _dir: TempDir,
    config: AppConfig,
    state: AppState,
    router: Router,
    actor: Identity,
    token: String,
}
impl TestApp {
    async fn new() -> Self {
        let dir = tempdir().unwrap();
        let mut config = AppConfig::default();
        config.auth.public_origin = "http://127.0.0.1:8080".into();
        config.download.public_base_url = "http://127.0.0.1:8080".into();
        config.database.path = dir.path().join("db.sqlite");
        config.storage.directory = dir.path().join("artifacts");
        config.storage.icons_directory = dir.path().join("icons");
        let pool = initialize(&config).await.unwrap();
        let state = application_state(pool, &config).await.unwrap();
        state
            .authentication
            .bootstrap("owner", "test-password-123".into())
            .await
            .unwrap();
        let login = state
            .authentication
            .login("owner", "test-password-123".into(), None)
            .await
            .unwrap();
        let actor = state.sessions.authenticate(&login.token).await.unwrap();
        let router = http::router(state.clone());
        Self {
            _dir: dir,
            config,
            state,
            router,
            actor,
            token: login.token,
        }
    }
    async fn request(
        &self,
        method: &str,
        path: &str,
        bytes: Vec<u8>,
        token: Option<&str>,
        csrf: bool,
    ) -> Response {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json")
            .header("origin", "http://127.0.0.1:8080")
            .header("x-veruna-request", "1");
        if let Some(token) = token {
            request = request.header("cookie", format!("veruna_session={token}"));
            if csrf {
                request = request.header("x-csrf-token", secrets::csrf(token));
            }
        }
        self.router
            .clone()
            .oneshot(request.body(Body::from(bytes)).unwrap())
            .await
            .unwrap()
    }
    async fn call(&self, method: &str, path: &str, value: Value, expected: u16) -> Value {
        let response = self
            .request(
                method,
                path,
                value.to_string().into_bytes(),
                Some(&self.token),
                true,
            )
            .await;
        assert_eq!(response.status(), expected, "{method} {path}");
        assert_eq!(response.headers()["cache-control"], "no-store");
        body(response).await
    }
    async fn create(&self, slug: &str, name: &str) -> Value {
        self.call("POST", BASE, json!({"slug":slug,"name":name}), 201)
            .await
    }
}
async fn body(response: Response) -> Value {
    serde_json::from_slice(&bytes(response).await).unwrap()
}
async fn bytes(response: Response) -> Vec<u8> {
    response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec()
}
fn path(app: &Value) -> String {
    format!("{BASE}/{}", app["id"].as_str().unwrap())
}
fn image(format: ImageFormat, width: u32) -> Vec<u8> {
    let mut output = Cursor::new(Vec::new());
    DynamicImage::new_rgb8(width, 2)
        .write_to(&mut output, format)
        .unwrap();
    output.into_inner()
}

#[tokio::test]
async fn catalog_lifecycle_preserves_identity_visibility_and_data_on_restart() {
    let app = TestApp::new().await;
    assert_eq!(app.call("GET", BASE, json!(null), 200).await["total"], 0);
    let created = app.create("prosto-crm", "Просто CRM").await;
    assert!(uuid::Uuid::parse_str(created["id"].as_str().unwrap()).is_ok());
    assert_eq!(created["status"], "active");
    assert_eq!(created["visibility"], "private");
    assert_eq!(created["description"], "");
    assert!(created["icon_url"].is_null());
    assert!(created.get("icon_key").is_none());
    assert_eq!(
        created,
        app.call("GET", &path(&created), json!(null), 200).await
    );
    let edited = app.call("PATCH", &path(&created), json!({"name":" Новый продукт ","description":"Описание\nприложения","visibility":"public"}), 200).await;
    assert_eq!(edited["name"], "Новый продукт");
    assert_eq!(edited["slug"], "prosto-crm");
    assert_eq!(edited["id"], created["id"]);
    assert_eq!(edited["created_at"], created["created_at"]);
    let archived = app
        .call(
            "POST",
            &format!("{}/archive", path(&created)),
            json!({}),
            200,
        )
        .await;
    assert_eq!(archived["status"], "archived");
    assert_eq!(archived["visibility"], "public");
    assert_eq!(
        archived,
        app.call(
            "POST",
            &format!("{}/archive", path(&created)),
            json!({}),
            200
        )
        .await
    );
    assert_eq!(app.call("GET", BASE, json!(null), 200).await["total"], 0);
    assert_eq!(
        app.call("GET", &format!("{BASE}?status=archived"), json!(null), 200)
            .await["items"][0],
        archived
    );
    assert_eq!(
        app.call(
            "POST",
            BASE,
            json!({"slug":"prosto-crm","name":"Duplicate"}),
            409
        )
        .await["error"]["code"],
        "slug_exists"
    );
    let restored = app
        .call(
            "POST",
            &format!("{}/restore", path(&created)),
            json!({}),
            200,
        )
        .await;
    assert_eq!(restored["status"], "active");
    assert_eq!(restored["visibility"], "public");
    app.call("DELETE", &path(&created), json!(null), 405).await;
    app.state.database.close().await;
    let pool = initialize(&app.config).await.unwrap();
    let state = application_state(pool, &app.config).await.unwrap();
    let stored = state
        .applications
        .get(created["id"].as_str().unwrap())
        .await
        .unwrap();
    assert_eq!(stored.slug, "prosto-crm");
    assert_eq!(stored.name, "Новый продукт");
    assert_eq!(stored.visibility, Visibility::Public);
}
#[tokio::test]
async fn validation_and_immutable_fields_are_enforced_by_api_and_database() {
    let app = TestApp::new().await;
    for slug in RESERVED_SLUGS.iter().copied().chain([
        "",
        "UPPER",
        "кириллица",
        "-start",
        "end-",
        "a/b",
        "a_b",
        "a.b",
        " white",
        "a%20",
        &"x".repeat(65),
    ]) {
        app.call("POST", BASE, json!({"slug":slug,"name":"Name"}), 400)
            .await;
    }
    for value in [
        json!({}),
        json!({"slug":"test"}),
        json!({"name":"Name"}),
        json!({"slug":"test","name":"  "}),
        json!({"slug":"test","name":"x".repeat(121)}),
        json!({"slug":"test","name":"Name","description":"x".repeat(2001)}),
        json!({"slug":"test","name":"Name","visibility":"unlisted"}),
        json!({"slug":"test","name":"Name","id":"forged"}),
        json!({"slug":"test","name":"Name","status":"archived"}),
        json!({"slug":"test","name":"Bad\u{0000}name"}),
    ] {
        app.call("POST", BASE, value, 400).await;
    }
    let created = app.create("a", "Name").await;
    for patch in [
        json!({"id":uuid::Uuid::new_v4().to_string()}),
        json!({"slug":"renamed"}),
        json!({"slug":"a"}),
        json!({"status":"archived"}),
        json!({"icon_key":"../escape"}),
        json!({}),
        json!({"name":null}),
        json!({"description":null}),
        json!({"visibility":null}),
        json!({"name":""}),
    ] {
        app.call("PATCH", &path(&created), patch, 400).await;
    }
    for (column, value) in [
        ("slug", "different"),
        ("id", "00000000-0000-0000-0000-000000000000"),
        ("created_at", "1"),
        ("status", "deleted"),
        ("visibility", "unlisted"),
    ] {
        assert!(
            sqlx::query(&format!("UPDATE applications SET {column}=? WHERE id=?"))
                .bind(value)
                .bind(created["id"].as_str().unwrap())
                .execute(&app.state.database)
                .await
                .is_err()
        );
    }
    app.call(
        "GET",
        &format!("{BASE}/{}", uuid::Uuid::new_v4()),
        json!(null),
        404,
    )
    .await;
    app.call("GET", &format!("{BASE}/bad-id"), json!(null), 404)
        .await;
    let response = app
        .request("POST", BASE, b"{".to_vec(), Some(&app.token), true)
        .await;
    assert_eq!(response.status(), 400);
    assert_eq!(body(response).await["error"]["code"], "invalid_application");
}
#[tokio::test]
async fn search_filters_sorting_and_pagination_are_stable_and_literal() {
    let app = TestApp::new().await;
    let a = app.create("alpha", "Альфа %_").await;
    let b = app.create("beta", "Бета").await;
    app.create("gamma", "Гамма").await;
    app.call("PATCH", &path(&a), json!({"visibility":"public"}), 200)
        .await;
    app.call("POST", &format!("{}/archive", path(&b)), json!({}), 200)
        .await;
    let page = app
        .call(
            "GET",
            &format!("{BASE}?status=all&sort=name&order=asc&per_page=1&page=2"),
            json!(null),
            200,
        )
        .await;
    assert_eq!(page["total"], 3);
    assert_eq!(page["items"].as_array().unwrap().len(), 1);
    assert_eq!(page["items"][0]["slug"], "beta");
    assert_eq!(
        app.call(
            "GET",
            &format!("{BASE}?search=%D0%90%D0%9B%D0%AC%D0%A4%D0%90"),
            json!(null),
            200
        )
        .await["items"][0]["id"],
        a["id"]
    );
    assert_eq!(
        app.call(
            "GET",
            &format!("{BASE}?search=ALP&visibility=public"),
            json!(null),
            200
        )
        .await["total"],
        1
    );
    assert_eq!(
        app.call("GET", &format!("{BASE}?search=%25_"), json!(null), 200)
            .await["total"],
        1
    );
    assert_eq!(
        app.call(
            "GET",
            &format!("{BASE}?search=%27%20OR%201=1--"),
            json!(null),
            200
        )
        .await["total"],
        0
    );
    assert_eq!(
        app.call(
            "GET",
            &format!("{BASE}?status=archived&visibility=private"),
            json!(null),
            200
        )
        .await["items"][0]["id"],
        b["id"]
    );
    assert_eq!(
        app.call("GET", &format!("{BASE}?page=100"), json!(null), 200)
            .await["items"],
        json!([])
    );
    for query in [
        "page=0",
        "page=-1",
        "per_page=0",
        "per_page=101",
        "status=deleted",
        "visibility=other",
        "sort=slug",
        "order=invalid",
        "page=abc",
        "unknown=true",
        "page=1&page=2",
        "sort=name%3BDELETE",
    ] {
        app.call("GET", &format!("{BASE}?{query}"), json!(null), 400)
            .await;
    }
}
#[tokio::test]
async fn role_csrf_session_and_method_boundaries_cover_catalog_and_icons() {
    let app = TestApp::new().await;
    let created = app.create("app", "Application").await;
    app.state
        .users
        .create(
            &app.actor,
            "operator",
            Role::Operator,
            Some("test-password-123".into()),
        )
        .await
        .unwrap();
    let operator = app
        .state
        .authentication
        .login("operator", "test-password-123".into(), None)
        .await
        .unwrap();
    for (method, route) in [
        ("GET", BASE.to_owned()),
        ("POST", BASE.to_owned()),
        ("GET", path(&created)),
        ("PATCH", path(&created)),
        ("POST", format!("{}/archive", path(&created))),
        ("POST", format!("{}/restore", path(&created))),
        ("POST", format!("{}/icon", path(&created))),
        ("DELETE", format!("{}/icon", path(&created))),
        ("GET", format!("{}/icon", path(&created))),
    ] {
        let response = app
            .request(method, &route, b"{}".to_vec(), None, true)
            .await;
        assert_eq!(response.status(), 401, "{method} {route}");
        let response = app
            .request(method, &route, b"{}".to_vec(), Some(&operator.token), true)
            .await;
        let expected = if method == "GET" {
            if route.ends_with("/icon") { 404 } else { 200 }
        } else {
            403
        };
        assert_eq!(response.status(), expected, "{method} {route}");
        assert!(
            response.headers()["content-type"]
                .to_str()
                .unwrap()
                .contains("json")
        );
        if method != "GET" {
            assert_eq!(
                app.request(method, &route, b"{}".to_vec(), Some(&app.token), false)
                    .await
                    .status(),
                403
            );
        }
    }
    assert_eq!(
        app.request("PUT", BASE, Vec::new(), None, true)
            .await
            .status(),
        401
    );
    assert_eq!(
        app.request("PUT", BASE, Vec::new(), Some(&operator.token), true)
            .await
            .status(),
        StatusCode::METHOD_NOT_ALLOWED
    );
    let request = Request::builder()
        .method("POST")
        .uri(format!("{}/archive", path(&created)))
        .header("origin", "https://evil.test")
        .header("x-veruna-request", "1")
        .header("x-csrf-token", secrets::csrf(&app.token))
        .header("cookie", format!("veruna_session={}", app.token))
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app.router.clone().oneshot(request).await.unwrap().status(),
        403
    );
}
#[tokio::test]
async fn concurrent_slug_creation_and_stale_identity_cannot_bypass_invariants() {
    let app = TestApp::new().await;
    let make = || NewApplication {
        slug: "unique".into(),
        name: "Name".into(),
        description: String::new(),
        visibility: Visibility::Private,
    };
    let (a, b) = tokio::join!(
        app.state.applications.create(&app.actor, make()),
        app.state.applications.create(&app.actor, make())
    );
    let stored = match (a, b) {
        (Ok(a), Err(ServiceError::ApplicationSlugConflict))
        | (Err(ServiceError::ApplicationSlugConflict), Ok(a)) => a,
        _ => panic!("Only one concurrent creation must succeed"),
    };
    app.state.sessions.logout(&app.actor).await.unwrap();
    assert!(matches!(
        app.state.applications.create(&app.actor, make()).await,
        Err(ServiceError::Unauthorized)
    ));
    assert!(matches!(
        app.state
            .applications
            .update(
                &app.actor,
                &stored.id,
                ApplicationChanges {
                    name: Some("forged".into()),
                    ..Default::default()
                }
            )
            .await,
        Err(ServiceError::Unauthorized)
    ));
    assert!(matches!(
        app.state
            .applications
            .set_status(&app.actor, &stored.id, Status::Archived)
            .await,
        Err(ServiceError::Unauthorized)
    ));
    assert!(matches!(
        app.state
            .applications
            .set_icon(&app.actor, &stored.id, Some(image(ImageFormat::Png, 2)))
            .await,
        Err(ServiceError::Unauthorized)
    ));
    let unchanged = app.state.applications.get(&stored.id).await.unwrap();
    assert_eq!(unchanged.name, "Name");
    assert_eq!(unchanged.status, Status::Active);
    assert!(unchanged.icon_key.is_none());
}
#[tokio::test]
async fn icon_content_limits_replacement_access_and_startup_recovery() {
    let app = TestApp::new().await;
    let created = app.create("icon-test", "Icons").await;
    let icon_path = format!("{}/icon", path(&created));
    let mut urls = Vec::new();
    for format in [ImageFormat::Png, ImageFormat::Jpeg, ImageFormat::WebP] {
        // Deliberately use application/json: content, not the supplied MIME, decides.
        let response = app
            .request("POST", &icon_path, image(format, 3), Some(&app.token), true)
            .await;
        assert_eq!(response.status(), 200);
        let updated = body(response).await;
        urls.push(updated["icon_url"].as_str().unwrap().to_owned());
        let response = app
            .request(
                "GET",
                urls.last().unwrap(),
                Vec::new(),
                Some(&app.token),
                true,
            )
            .await;
        assert_eq!(response.status(), 200);
        assert_eq!(response.headers()["content-type"], "image/png");
        assert_eq!(response.headers()["cache-control"], "no-store");
        assert_eq!(response.headers()["x-content-type-options"], "nosniff");
        let decoded = image::load_from_memory(&bytes(response).await).unwrap();
        assert_eq!(decoded.width(), 3);
    }
    assert_ne!(urls[0], urls[1]);
    assert_ne!(urls[1], urls[2]);
    let last = app.call("GET", &path(&created), json!(null), 200).await;
    for bytes in [
        b"<svg xmlns='http://www.w3.org/2000/svg'/>".to_vec(),
        b"GIF89a".to_vec(),
        b"\x89PNG\r\n\x1a\n".to_vec(),
        Vec::new(),
        vec![0; MAX_ICON_BYTES + 1],
        image(ImageFormat::Png, 1025),
    ] {
        let response = app
            .request("POST", &icon_path, bytes, Some(&app.token), true)
            .await;
        assert_eq!(response.status(), 400);
        assert_eq!(body(response).await["error"]["code"], "invalid_icon");
        assert_eq!(
            app.call("GET", &path(&created), json!(null), 200).await["icon_url"],
            last["icon_url"]
        );
    }
    let deleted = app.call("DELETE", &icon_path, json!(null), 200).await;
    assert!(deleted["icon_url"].is_null());
    app.call("DELETE", &icon_path, json!(null), 200).await;
    app.call("GET", &icon_path, json!(null), 404).await;
    let response = app
        .request(
            "POST",
            &icon_path,
            image(ImageFormat::Png, 2),
            Some(&app.token),
            true,
        )
        .await;
    let current = body(response).await;
    let key: String = sqlx::query_scalar("SELECT icon_key FROM applications WHERE id=?")
        .bind(created["id"].as_str().unwrap())
        .fetch_one(&app.state.database)
        .await
        .unwrap();
    let temporary = app
        .config
        .storage
        .icons_directory
        .join(format!("{}.tmp", uuid::Uuid::new_v4().simple()));
    std::fs::write(&temporary, b"partial").unwrap();
    app.state.database.close().await;
    let pool = initialize(&app.config).await.unwrap();
    assert!(!temporary.exists());
    let files: Vec<_> = std::fs::read_dir(&app.config.storage.icons_directory)
        .unwrap()
        .map(|v| v.unwrap().file_name())
        .collect();
    assert_eq!(files, vec![std::ffi::OsString::from(key)]);
    let state = application_state(pool, &app.config).await.unwrap();
    assert_eq!(
        image::load_from_memory(
            &state
                .applications
                .icon(created["id"].as_str().unwrap())
                .await
                .unwrap()
        )
        .unwrap()
        .to_rgba8(),
        image::load_from_memory(&image(ImageFormat::Png, 2))
            .unwrap()
            .to_rgba8()
    );
    assert!(
        current["icon_url"]
            .as_str()
            .unwrap()
            .starts_with(&icon_path)
    );
}
#[cfg(unix)]
#[tokio::test]
async fn icon_storage_rejects_paths_and_symlinks_and_keeps_artifacts_separate() {
    let app = TestApp::new().await;
    let storage = IconStorage::new(app.config.storage.icons_directory.clone());
    let outside = app._dir.path().join("secret.png");
    std::fs::write(&outside, b"private").unwrap();
    let key = format!("{}.png", uuid::Uuid::new_v4().simple());
    std::os::unix::fs::symlink(&outside, app.config.storage.icons_directory.join(&key)).unwrap();
    assert!(storage.read(&key).await.is_err());
    for key in ["../secret.png", "/etc/passwd", "..%2fsecret.png", "bad.png"] {
        assert!(storage.read(key).await.is_err());
    }
    storage
        .initialize(app.config.storage.directory.clone(), Vec::new())
        .await
        .unwrap();
    assert_eq!(std::fs::read(&outside).unwrap(), b"private");
    assert!(
        IconStorage::new(app.config.storage.directory.join("icons"))
            .initialize(app.config.storage.directory.clone(), Vec::new())
            .await
            .is_err()
    );
}
