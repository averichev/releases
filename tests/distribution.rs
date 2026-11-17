use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
    response::Response,
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tempfile::{TempDir, tempdir};
use tower::ServiceExt;
use veruna_releases::{
    application_state,
    config::{AppConfig, DeliveryMode},
    domain::{
        application::{
            ApplicationChanges, DistributionSelector, NewApplication, Status, Visibility,
        },
        artifact::{Artifact, UploadMetadata},
        release::{Release, ReleaseInput},
        user::Role,
    },
    http::{self, AppState},
    infrastructure::{
        secrets,
        storage::{ArtifactStorage, LocalArtifactStorage},
    },
    initialize,
    services::{publishing::PublicationCommand, sessions::Identity},
};
struct TestApp {
    dir: TempDir,
    config: AppConfig,
    state: AppState,
    router: Router,
    actor: Identity,
    token: String,
    app: String,
}
impl TestApp {
    async fn new(mode: DeliveryMode) -> Self {
        let dir = tempdir().unwrap();
        let mut config = AppConfig::default();
        config.database.path = dir.path().join("db.sqlite");
        config.storage.directory = dir.path().join("objects");
        config.storage.icons_directory = dir.path().join("icons");
        config.download.public_base_url = "https://apps.example.test".into();
        config.download.delivery_mode = mode;
        config.download.max_concurrent_downloads = 1;
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
        let app = state
            .applications
            .create(
                &actor,
                NewApplication {
                    slug: "product".into(),
                    name: "Приложение".into(),
                    description: "Описание".into(),
                    visibility: Visibility::Public,
                },
            )
            .await
            .unwrap()
            .id;
        let router = http::router(state.clone());
        Self {
            dir,
            config,
            state,
            router,
            actor,
            token: login.token,
            app,
        }
    }
    async fn create(&self, version: &str) -> Release {
        self.state
            .releases
            .create(
                &self.actor,
                &self.app,
                ReleaseInput {
                    version: version.into(),
                    title: None,
                    release_notes: "Что нового".into(),
                },
            )
            .await
            .unwrap()
    }
    async fn upload(&self, r: &Release, target: &str, name: &str) -> Artifact {
        let mut upload = self
            .state
            .artifacts
            .begin(
                &self.actor,
                &self.app,
                &r.id,
                UploadMetadata {
                    filename: name.into(),
                    target: target.into(),
                    file_type: None,
                },
            )
            .await
            .unwrap();
        upload.write(b"0123456789").await.unwrap();
        self.state
            .artifacts
            .complete(&self.actor, upload)
            .await
            .unwrap()
    }
    async fn publish(&self, r: &Release, name: &str) {
        let channel = self
            .state
            .publishing
            .channels(&self.app)
            .await
            .unwrap()
            .into_iter()
            .find(|c| c.name == name)
            .unwrap();
        let current = self.state.releases.get(&self.app, &r.id).await.unwrap();
        self.state
            .publishing
            .publish(
                &self.actor,
                &self.app,
                &r.id,
                PublicationCommand {
                    channel: name.into(),
                    expected_channel_revision: channel.revision,
                    expected_release_revision: current.revision,
                },
            )
            .await
            .unwrap();
    }
    async fn get(&self, method: &str, path: &str, headers: &[(&str, &str)]) -> Response {
        let mut req = Request::builder()
            .method(method)
            .uri(path)
            .header("host", "hostile.example");
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        self.router
            .clone()
            .oneshot(req.body(Body::empty()).unwrap())
            .await
            .unwrap()
    }
    async fn admin(
        &self,
        method: &str,
        path: &str,
        body: Value,
        token: Option<&str>,
        csrf: bool,
    ) -> Response {
        let mut req = Request::builder()
            .method(method)
            .uri(path)
            .header("origin", &self.config.auth.public_origin)
            .header("x-veruna-request", "1")
            .header("content-type", "application/json");
        if let Some(token) = token {
            req = req.header("cookie", format!("veruna_session={token}"));
            if csrf {
                req = req.header("x-csrf-token", secrets::csrf(token));
            }
        }
        self.router
            .clone()
            .oneshot(req.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap()
    }
    fn url(&self, r: &Release, a: &Artifact, channel: &str) -> String {
        self.state
            .distribution
            .artifact_path("product", r, a, channel)
    }
}
async fn bytes(r: Response) -> Vec<u8> {
    r.into_body().collect().await.unwrap().to_bytes().to_vec()
}
async fn json_body(r: Response) -> Value {
    serde_json::from_slice(&bytes(r).await).unwrap()
}
fn signature() -> String {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let mut packet = [0; 74];
    packet[..2].copy_from_slice(b"ED");
    // A syntactically valid test signature, never distributed as a real package.
    STANDARD.encode(format!(
        "untrusted comment: test\n{}\ntrusted comment: test\n{}\n",
        STANDARD.encode(packet),
        STANDARD.encode([0; 64])
    ))
}
#[tokio::test]
async fn shared_resolution_selectors_defaults_and_channel_rollback() {
    let t = TestApp::new(DeliveryMode::Direct).await;
    assert_eq!(t.get("GET", "/product/latest", &[]).await.status(), 404);
    let r = t.create("1.0.0").await;
    let win = t.upload(&r, "windows-x86_64", "setup.exe").await;
    let linux = t.upload(&r, "linux-x86_64", "app.AppImage").await;
    t.publish(&r, "stable").await;
    assert_eq!(t.get("GET", "/product/latest", &[]).await.status(), 409);
    assert_eq!(
        t.get("GET", "/product/latest?target=windows-x86_64", &[])
            .await
            .headers()["location"],
        t.url(&r, &win, "stable")
    );
    assert_eq!(
        t.get("GET", "/product/latest?format=appimage", &[])
            .await
            .headers()["location"],
        t.url(&r, &linux, "stable")
    );
    assert_eq!(
        t.get("GET", "/product/latest?target=darwin-aarch64", &[])
            .await
            .status(),
        404
    );
    t.state
        .applications
        .set_distribution(
            &t.actor,
            &t.app,
            Some(DistributionSelector {
                target: win.target.clone(),
                format: win.file_type.clone(),
                purpose: "installer".into(),
            }),
        )
        .await
        .unwrap();
    let latest = t.get("GET", "/product/latest", &[]).await;
    assert_eq!(latest.status(), 307);
    assert_eq!(latest.headers()["cache-control"], "no-store");
    assert_eq!(latest.headers()["location"], t.url(&r, &win, "stable"));
    assert_eq!(
        t.get("GET", "/product/1.0.0/linux-x86_64", &[])
            .await
            .headers()["location"],
        t.url(&r, &linux, "stable")
    );
    let beta = t.create("2.0.0-beta.1").await;
    let b = t.upload(&beta, "windows-x86_64", "beta.exe").await;
    t.publish(&beta, "beta").await;
    assert_eq!(
        t.get("GET", "/product/latest?channel=beta", &[])
            .await
            .headers()["location"],
        t.url(&beta, &b, "beta")
    );
    assert_eq!(
        t.get("GET", &t.url(&beta, &b, "stable"), &[])
            .await
            .status(),
        404
    );
    assert_eq!(
        t.get("GET", &t.url(&beta, &b, "beta"), &[]).await.status(),
        200
    );
    let r2 = t.create("1.1.0").await;
    let a2 = t.upload(&r2, "windows-x86_64", "new.exe").await;
    t.publish(&r2, "stable").await;
    assert_eq!(
        t.get("GET", "/product/latest", &[]).await.headers()["location"],
        t.url(&r2, &a2, "stable")
    );
    t.publish(&r, "stable").await;
    assert_eq!(
        t.get("GET", "/product/latest", &[]).await.headers()["location"],
        t.url(&r, &win, "stable")
    );
    assert_eq!(
        bytes(t.get("GET", &t.url(&r2, &a2, "stable"), &[]).await).await,
        b"0123456789"
    );
}
#[tokio::test]
async fn explicit_format_required_and_updater_never_implicitly_selected() {
    let t = TestApp::new(DeliveryMode::Direct).await;
    let r = t.create("1.0.0").await;
    let a = t.upload(&r, "windows-x86_64", "setup.exe").await;
    let m = t.upload(&r, "windows-x86_64", "setup.msi").await;
    t.state
        .artifacts
        .set_updater(
            &t.actor,
            &t.app,
            &r.id,
            &a.id,
            Some("updater"),
            Some(Some(signature().as_bytes())),
        )
        .await
        .unwrap();
    t.publish(&r, "stable").await;
    assert_eq!(
        t.get("GET", "/product/latest", &[]).await.headers()["location"],
        t.url(&r, &m, "stable")
    );
    assert_eq!(
        t.get("GET", "/product/latest?purpose=updater", &[])
            .await
            .headers()["location"],
        t.url(&r, &a, "stable")
    );
    assert_eq!(
        t.get("GET", "/product/latest?format=exe", &[])
            .await
            .status(),
        404
    );
    let r2 = t.create("1.1.0").await;
    t.upload(&r2, "windows-x86_64", "setup.exe").await;
    t.upload(&r2, "windows-x86_64", "setup.msi").await;
    t.publish(&r2, "stable").await;
    assert_eq!(
        t.get("GET", "/product/1.1.0/windows-x86_64", &[])
            .await
            .status(),
        409
    );
    assert_eq!(
        t.get("GET", "/product/1.1.0/windows-x86_64?format=exe", &[])
            .await
            .status(),
        307
    );
}
#[tokio::test]
async fn get_head_ranges_validators_unicode_and_cancellation() {
    let t = TestApp::new(DeliveryMode::Direct).await;
    let r = t.create("1.0.0").await;
    let a = t.upload(&r, "windows-x86_64", "Приложение \"1\".exe").await;
    t.publish(&r, "stable").await;
    let url = t.url(&r, &a, "stable");
    let head = t.get("HEAD", &url, &[]).await;
    assert_eq!(head.status(), 200);
    assert_eq!(head.headers()["content-length"], "10");
    assert_eq!(head.headers()["accept-ranges"], "bytes");
    assert!(
        head.headers()["content-disposition"]
            .to_str()
            .unwrap()
            .contains("filename*=UTF-8''%D0")
    );
    let etag = head.headers()["etag"].to_str().unwrap().to_owned();
    let modified = head.headers()["last-modified"].to_str().unwrap().to_owned();
    assert!(bytes(head).await.is_empty());
    let response = t.get("GET", &url, &[("accept-encoding", "gzip")]).await;
    assert!(!response.headers().contains_key("content-encoding"));
    assert_eq!(t.get("GET", &url, &[]).await.status(), 503);
    drop(response);
    assert_eq!(bytes(t.get("GET", &url, &[]).await).await, b"0123456789");
    for (range, content_range, expected) in [
        ("bytes=0-3", "bytes 0-3/10", "0123"),
        ("bytes=7-", "bytes 7-9/10", "789"),
        ("bytes=-3", "bytes 7-9/10", "789"),
        ("bytes=9-9", "bytes 9-9/10", "9"),
        ("bytes=0-99", "bytes 0-9/10", "0123456789"),
        ("bytes=-99", "bytes 0-9/10", "0123456789"),
    ] {
        let response = t
            .get("GET", &url, &[("range", range), ("if-range", &etag)])
            .await;
        assert_eq!(response.status(), 206, "{range}");
        assert_eq!(response.headers()["content-range"], content_range);
        assert_eq!(
            response.headers()["content-length"],
            expected.len().to_string()
        );
        assert_eq!(bytes(response).await, expected.as_bytes());
    }
    for range in [
        "bytes=10-",
        "bytes=-0",
        "bytes=8-3",
        "bytes=999999999999999999999999999-",
        "bytes=oops",
    ] {
        let response = t.get("GET", &url, &[("range", range)]).await;
        assert_eq!(response.status(), 416);
        assert_eq!(response.headers()["content-range"], "bytes */10");
        assert!(bytes(response).await.is_empty());
    }
    for headers in [
        vec![("range", "bytes=1-2"), ("if-range", "\"old\"")],
        vec![("range", "bytes=0-1,4-5")],
        vec![("range", "items=1-2")],
    ] {
        let response = t.get("GET", &url, &headers).await;
        assert_eq!(response.status(), 200);
        assert_eq!(bytes(response).await.len(), 10);
    }
    assert_eq!(
        t.get("HEAD", &url, &[("range", "bytes=1-2")])
            .await
            .status(),
        200
    );
    assert_eq!(
        t.get(
            "GET",
            &url,
            &[("range", "bytes=1-2"), ("if-range", &modified)]
        )
        .await
        .status(),
        206
    );
    for headers in [
        vec![("if-none-match", etag.as_str())],
        vec![("if-modified-since", modified.as_str())],
        vec![("if-none-match", "*")],
    ] {
        let res = t.get("GET", &url, &headers).await;
        assert_eq!(res.status(), 304);
        assert!(bytes(res).await.is_empty());
    }
    assert_eq!(
        t.get("GET", &url, &[("if-match", "\"old\"")])
            .await
            .status(),
        412
    );
}
#[tokio::test]
async fn public_catalog_pagination_real_urls_and_all_privacy_boundaries() {
    let t = TestApp::new(DeliveryMode::Direct).await;
    let r = t.create("1.0.0").await;
    let a = t.upload(&r, "windows-x86_64", "setup.exe").await;
    let url = t.url(&r, &a, "stable");
    assert_eq!(t.get("GET", &url, &[]).await.status(), 404);
    let empty = json_body(
        t.get("GET", "/api/v1/public/applications/product", &[])
            .await,
    )
    .await;
    assert!(empty["latest"].is_null());
    t.publish(&r, "stable").await;
    let catalog = json_body(
        t.get("GET", "/api/v1/public/applications/product", &[])
            .await,
    )
    .await;
    assert_eq!(catalog["latest"]["version"], "1.0.0");
    assert_eq!(
        catalog["latest"]["artifacts"][0]["download_url"],
        format!("https://apps.example.test{url}")
    );
    let manifest = json_body(t.get("GET", "/product/manifest.json", &[]).await).await;
    assert_eq!(
        manifest["releases"][0]["artifacts"][0]["download_url"],
        format!("https://apps.example.test{url}")
    );
    assert!(!catalog.to_string().contains("storage_key"));
    assert!(!catalog.to_string().contains("created_at"));
    let b = t.create("2.0.0-beta.1").await;
    t.upload(&b, "linux-x86_64", "beta.AppImage").await;
    t.publish(&b, "beta").await;
    let history = json_body(
        t.get(
            "GET",
            "/api/v1/public/applications/product/releases?limit=1&offset=1",
            &[],
        )
        .await,
    )
    .await;
    assert_eq!(history["total"], 1);
    assert_eq!(history["items"], json!([]));
    assert_eq!(
        t.get(
            "GET",
            "/api/v1/public/applications/product/releases?limit=101",
            &[]
        )
        .await
        .status(),
        400
    );
    assert_eq!(
        t.get(
            "GET",
            "/api/v1/public/applications/product/releases?channel=no",
            &[]
        )
        .await
        .status(),
        400
    );
    let other = t
        .state
        .applications
        .create(
            &t.actor,
            NewApplication {
                slug: "other".into(),
                name: "Other".into(),
                description: String::new(),
                visibility: Visibility::Public,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        t.get("GET", &url.replacen("/product/", "/other/", 1), &[])
            .await
            .status(),
        404
    );
    for visibility in [Visibility::Private, Visibility::Public] {
        t.state
            .applications
            .update(
                &t.actor,
                &t.app,
                ApplicationChanges {
                    visibility: Some(visibility),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        if visibility == Visibility::Public {
            t.state
                .applications
                .set_status(&t.actor, &t.app, Status::Archived)
                .await
                .unwrap();
        }
        for path in [
            &url,
            "/product",
            "/product/latest",
            "/product/manifest.json",
            "/api/v1/public/applications/product",
            "/api/v1/public/applications/product/releases",
            "/api/v1/updates/tauri/product/stable/windows/x86_64/0.0.0",
        ] {
            assert_eq!(
                t.get("GET", path, &[("if-none-match", "*")]).await.status(),
                404,
                "{path}"
            );
        }
    }
    assert_ne!(other.id, t.app);
}
#[tokio::test]
async fn missing_files_symlinks_traversal_and_internal_location_are_never_served() {
    let t = TestApp::new(DeliveryMode::Direct).await;
    let r = t.create("1.0.0").await;
    let a = t.upload(&r, "windows-x86_64", "setup.exe").await;
    t.publish(&r, "stable").await;
    let path = t.config.storage.directory.join(&a.storage_key);
    tokio::fs::remove_file(&path).await.unwrap();
    let response = t.get("GET", &t.url(&r, &a, "stable"), &[]).await;
    assert_eq!(response.status(), 503);
    assert!(
        !String::from_utf8(bytes(response).await)
            .unwrap()
            .contains(t.dir.path().to_str().unwrap())
    );
    #[cfg(unix)]
    {
        let secret = t.dir.path().join("outside");
        std::fs::write(&secret, b"0123456789").unwrap();
        std::os::unix::fs::symlink(secret, &path).unwrap();
        assert_eq!(
            t.get("HEAD", &t.url(&r, &a, "stable"), &[]).await.status(),
            503
        );
    }
    for path in [
        "/_veruna_files/abc.blob",
        "/_veruna_files/%2e%2e/db.sqlite",
        "/product/releases/1.0.0/artifacts/%2e%2e%2fdb/download",
        "/product%2f..%2fapi/latest",
        "/product/%2e%2e/windows-x86_64",
        "/product/releases/1.0.0/artifacts/%252e%252e/download",
    ] {
        assert_eq!(t.get("GET", path, &[]).await.status(), 404, "{path}");
    }
    let storage = LocalArtifactStorage::new(&t.config.storage);
    assert!(storage.internal_uri("../outside").is_err());
}
#[tokio::test]
async fn nginx_delegation_is_authorized_and_head_is_complete_without_body() {
    let t = TestApp::new(DeliveryMode::Nginx).await;
    let r = t.create("1.0.0").await;
    let a = t.upload(&r, "windows-x86_64", "setup.exe").await;
    t.publish(&r, "stable").await;
    let response = t
        .get("GET", &t.url(&r, &a, "stable"), &[("range", "bytes=1-3")])
        .await;
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.headers()["x-accel-redirect"],
        format!("/_veruna_files/{}", a.storage_key)
    );
    assert!(bytes(response).await.is_empty());
    let head = t.get("HEAD", &t.url(&r, &a, "stable"), &[]).await;
    assert_eq!(head.headers()["content-length"], "10");
    assert!(!head.headers().contains_key("x-accel-redirect"));
    assert!(bytes(head).await.is_empty());
    let invalid = t
        .get("GET", &t.url(&r, &a, "stable"), &[("range", "bytes=100-")])
        .await;
    assert_eq!(invalid.status(), 416);
    assert!(!invalid.headers().contains_key("x-accel-redirect"));
    t.state
        .applications
        .set_status(&t.actor, &t.app, Status::Archived)
        .await
        .unwrap();
    assert_eq!(
        t.get("GET", &t.url(&r, &a, "stable"), &[]).await.status(),
        404
    );
}
#[tokio::test]
async fn tauri_http_updates_signatures_channels_and_no_downgrades() {
    let t = TestApp::new(DeliveryMode::Direct).await;
    let endpoint = "/api/v1/updates/tauri/product/stable/windows/x86_64/1.0.0";
    let empty = t.get("GET", endpoint, &[]).await;
    assert_eq!(empty.status(), 204);
    assert!(bytes(empty).await.is_empty());
    let r = t.create("1.2.0").await;
    let a = t.upload(&r, "windows-x86_64", "setup.exe").await;
    let sig = signature();
    t.state
        .artifacts
        .set_updater(
            &t.actor,
            &t.app,
            &r.id,
            &a.id,
            Some("updater"),
            Some(Some(sig.as_bytes())),
        )
        .await
        .unwrap();
    t.publish(&r, "stable").await;
    let response = t.get("GET", endpoint, &[]).await;
    assert_eq!(response.status(), 200);
    let data = json_body(response).await;
    assert_eq!(data["signature"], sig);
    assert_eq!(data["version"], "1.2.0");
    assert_eq!(
        data["url"],
        format!("https://apps.example.test{}", t.url(&r, &a, "stable"))
    );
    for current in ["1.2.0", "1.2.0+build", "1.9.0"] {
        let response = t
            .get(
                "GET",
                &endpoint.replace("/1.0.0", &format!("/{current}")),
                &[],
            )
            .await;
        assert_eq!(response.status(), 204);
        assert!(bytes(response).await.is_empty());
    }
    for path in [
        endpoint.replace("/1.0.0", "/invalid"),
        endpoint.replace("/windows/", "/android/"),
        endpoint.replace("/x86_64/", "/unknown/"),
    ] {
        assert_eq!(t.get("GET", &path, &[]).await.status(), 422);
    }
    assert_eq!(
        t.get("GET", &endpoint.replace("windows", "linux"), &[])
            .await
            .status(),
        204
    );
    let beta = t.create("1.3.0-beta.1").await;
    let b = t.upload(&beta, "windows-x86_64", "beta.exe").await;
    t.state
        .artifacts
        .set_updater(
            &t.actor,
            &t.app,
            &beta.id,
            &b.id,
            Some("updater"),
            Some(Some(sig.as_bytes())),
        )
        .await
        .unwrap();
    t.publish(&beta, "beta").await;
    let beta_data = json_body(
        t.get("GET", &endpoint.replace("/stable/", "/beta/"), &[])
            .await,
    )
    .await;
    assert_eq!(beta_data["version"], "1.3.0-beta.1");
    assert!(
        beta_data["url"]
            .as_str()
            .unwrap()
            .ends_with("?channel=beta")
    );
    let unsigned = t.create("1.4.0").await;
    let u = t.upload(&unsigned, "windows-x86_64", "unsigned.exe").await;
    t.state
        .artifacts
        .set_updater(&t.actor, &t.app, &unsigned.id, &u.id, Some("updater"), None)
        .await
        .unwrap();
    t.publish(&unsigned, "stable").await;
    assert_eq!(t.get("GET", endpoint, &[]).await.status(), 503);
    assert_eq!(t.get("GET", "/product/latest", &[]).await.status(), 404);
    let preview = t
        .state
        .updater
        .preview(&t.app, "stable", "windows-x86_64", "1.0.0")
        .await
        .unwrap();
    assert_eq!(preview.reason, "signature_missing");
    t.publish(&r, "stable").await;
    assert_eq!(
        t.get("GET", &endpoint.replace("/1.0.0", "/1.4.0"), &[])
            .await
            .status(),
        204
    );
}
#[tokio::test]
async fn default_mutation_requires_admin_csrf_and_live_session_and_warns_on_publish() {
    let t = TestApp::new(DeliveryMode::Direct).await;
    let url = format!("/api/v1/admin/applications/{}/distribution/default", t.app);
    let body = json!({"target":"windows-x86_64","format":"exe","purpose":"installer"});
    assert_eq!(
        t.admin("PUT", &url, body.clone(), None, false)
            .await
            .status(),
        401
    );
    assert_eq!(
        t.admin("PUT", &url, body.clone(), Some(&t.token), false)
            .await
            .status(),
        403
    );
    let (operator_user, _) = t
        .state
        .users
        .create(
            &t.actor,
            "operator",
            Role::Operator,
            Some("test-password-456".into()),
        )
        .await
        .unwrap();
    let operator = t
        .state
        .authentication
        .login("operator", "test-password-456".into(), None)
        .await
        .unwrap();
    assert_eq!(
        t.admin("PUT", &url, body.clone(), Some(&operator.token), true)
            .await
            .status(),
        403
    );
    let settings = format!("/api/v1/admin/applications/{}/distribution", t.app);
    assert_eq!(t.get("GET", &settings, &[]).await.status(), 401);
    assert_eq!(
        t.admin("GET", &settings, Value::Null, Some(&operator.token), false)
            .await
            .status(),
        200
    );
    assert_eq!(
        t.admin("PUT", &url, body, Some(&t.token), true)
            .await
            .status(),
        204
    );
    let r = t.create("1.0.0").await;
    t.upload(&r, "linux-x86_64", "app.AppImage").await;
    let validation = t
        .state
        .publishing
        .validate(&t.actor, &t.app, &r.id, "stable")
        .await
        .unwrap();
    assert!(
        validation
            .validation
            .warnings
            .iter()
            .any(|i| i.code == "default_distribution_missing")
    );
    assert_eq!(
        t.admin(
            "PUT",
            &url,
            json!({"target":"windows-x86_64","format":"exe","purpose":"updater"}),
            Some(&t.token),
            true
        )
        .await
        .status(),
        422
    );
    assert_eq!(
        t.admin("PUT", &url, Value::Null, Some(&t.token), true)
            .await
            .status(),
        204
    );
    t.state
        .users
        .revoke(&t.actor, &operator_user.id)
        .await
        .unwrap();
    assert_eq!(
        t.admin("PUT", &url, Value::Null, Some(&operator.token), true)
            .await
            .status(),
        401
    );
    t.state
        .users
        .revoke(&t.actor, &t.actor.user.id)
        .await
        .unwrap();
    assert!(matches!(
        t.state
            .applications
            .set_distribution(&t.actor, &t.app, None)
            .await,
        Err(veruna_releases::services::error::ServiceError::Unauthorized)
    ));
}
#[tokio::test]
async fn sparse_file_over_two_gib_streams_and_reports_u64_metadata() {
    let t = TestApp::new(DeliveryMode::Direct).await;
    let r = t.create("1.0.0").await;
    let key = format!("{}.blob", uuid::Uuid::new_v4().simple());
    let id = uuid::Uuid::new_v4().to_string();
    let length = 3_u64 * 1024 * 1024 * 1024 + 1;
    let file = std::fs::File::create(t.config.storage.directory.join(&key)).unwrap();
    file.set_len(length).unwrap();
    drop(file);
    // Fixture bypasses publication hashing; this sparse object is never a real release.
    sqlx::query("INSERT INTO artifacts(id,release_id,filename,target,file_type,size_bytes,sha256,storage_key,created_at) VALUES(?,?,'large.bin','any','bin',?,?,?,?)").bind(&id).bind(&r.id).bind(length as i64).bind("0".repeat(64)).bind(&key).bind(r.created_at).execute(&t.state.database).await.unwrap();
    let mut tx = t.state.database.begin().await.unwrap();
    use veruna_releases::repositories::channel::ChannelsRepository;
    ChannelsRepository::publish(&mut tx, &r.id, secrets::now())
        .await
        .unwrap();
    let channel = ChannelsRepository::get_in(&mut tx, &t.app, "stable")
        .await
        .unwrap();
    ChannelsRepository::assign(&mut tx, &channel, &r.id, secrets::now())
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let url = format!("/product/releases/1.0.0/artifacts/{id}/download");
    let head = t.get("HEAD", &url, &[]).await;
    assert_eq!(head.headers()["content-length"], length.to_string());
    let mut response = t.get("GET", &url, &[]).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response
            .body_mut()
            .frame()
            .await
            .unwrap()
            .unwrap()
            .into_data()
            .unwrap()
            .len()
            <= 64 * 1024
    );
    drop(response);
    let last = t
        .get("GET", &url, &[("range", &format!("bytes={}-", length - 1))])
        .await;
    assert_eq!(last.status(), 206);
    assert_eq!(bytes(last).await, vec![0]);
}

fn public_key() -> String {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let mut packet = [0u8; 42];
    packet[..2].copy_from_slice(b"Ed");
    STANDARD.encode(format!(
        "untrusted comment: integration test public key\n{}\n",
        STANDARD.encode(packet)
    ))
}
#[test]
fn tauri_configuration_preserves_tokens_and_serializes_values_safely() {
    use veruna_releases::services::tauri_integration::{configuration, endpoint};
    let stable = endpoint("https://updates.example.test/", "product-one", "stable").unwrap();
    assert_eq!(
        stable,
        "https://updates.example.test/api/v1/updates/tauri/product-one/stable/{{target}}/{{arch}}/{{current_version}}"
    );
    let beta = endpoint("https://another.example.test", "second-app", "beta").unwrap();
    assert!(beta.starts_with("https://another.example.test/api/v1/updates/tauri/second-app/beta/"));
    let key = "quoted\"key\nwith\\escapes";
    let value: Value = serde_json::from_str(&configuration(key, &stable).to_string()).unwrap();
    assert_eq!(value["plugins"]["updater"]["pubkey"], key);
    assert_eq!(value["plugins"]["updater"]["endpoints"][0], stable);
    assert_eq!(value["bundle"]["createUpdaterArtifacts"], true);
    for base in [
        "https://evil.test/path",
        "https://user:password@example.test",
        "https://example.test?x=1",
        "file:///tmp",
    ] {
        assert!(endpoint(base, "product", "stable").is_err());
    }
    assert!(endpoint("https://example.test", "../bad", "stable").is_err());
    assert!(endpoint("https://example.test", "product", "nightly").is_err());
}
#[tokio::test]
async fn tauri_settings_key_persistence_validation_access_and_revoked_session() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let t = TestApp::new(DeliveryMode::Direct).await;
    let path = format!("/api/v1/admin/applications/{}/integrations/tauri", t.app);
    assert_eq!(
        t.admin("GET", &path, Value::Null, None, false)
            .await
            .status(),
        401
    );
    let initial = json_body(
        t.admin("GET", &path, Value::Null, Some(&t.token), false)
            .await,
    )
    .await;
    assert!(initial["public_key"].is_null());
    assert!(initial["configuration"].is_null());
    assert_eq!(initial["ready"], false);
    let codes: Vec<_> = initial["blockers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["code"].as_str().unwrap())
        .collect();
    for code in [
        "public_key_missing",
        "published_release_missing",
        "channel_release_missing",
    ] {
        assert!(codes.contains(&code));
    }
    t.state
        .users
        .create(
            &t.actor,
            "operator",
            Role::Operator,
            Some("test-password-123".into()),
        )
        .await
        .unwrap();
    let operator = t
        .state
        .authentication
        .login("operator", "test-password-123".into(), None)
        .await
        .unwrap()
        .token;
    assert_eq!(
        t.admin("GET", &path, Value::Null, Some(&operator), false)
            .await
            .status(),
        200
    );
    for (token, csrf, status) in [
        (None, true, 401),
        (Some(operator.as_str()), true, 403),
        (Some(t.token.as_str()), false, 403),
    ] {
        assert_eq!(
            t.admin(
                "PUT",
                &path,
                json!({"public_key":public_key()}),
                token,
                csrf
            )
            .await
            .status(),
            status
        );
    }
    for key in [
        "PRIVATE KEY".to_owned(),
        STANDARD.encode(format!(
            "untrusted comment: private key\n{}\n",
            STANDARD.encode([0; 158])
        )),
        "x".repeat(4097),
    ] {
        assert_eq!(
            t.admin(
                "PUT",
                &path,
                json!({"public_key":key}),
                Some(&t.token),
                true
            )
            .await
            .status(),
            422
        );
    }
    assert_eq!(
        t.admin(
            "PUT",
            &path,
            json!({"public_key":public_key(), "private_key":"never accepted"}),
            Some(&t.token),
            true
        )
        .await
        .status(),
        400
    );
    let key = public_key();
    assert_eq!(
        t.admin(
            "PUT",
            &path,
            json!({"public_key":format!("  {key}\n")}),
            Some(&t.token),
            true
        )
        .await
        .status(),
        204
    );
    // A newly composed application state reads the durable setting, not a cache.
    let restarted = application_state(t.state.database.clone(), &t.config)
        .await
        .unwrap();
    let d = restarted
        .tauri_integration
        .details(&t.app, "beta", false)
        .await
        .unwrap();
    assert_eq!(d.public_key.as_deref(), Some(key.as_str()));
    assert!(
        d.endpoint
            .contains("https://apps.example.test/api/v1/updates/tauri/product/beta/{{target}}")
    );
    assert_eq!(
        d.configuration.unwrap()["plugins"]["updater"]["pubkey"],
        key
    );
    assert_eq!(
        t.admin(
            "GET",
            &format!("{path}?channel=bad"),
            Value::Null,
            Some(&t.token),
            false
        )
        .await
        .status(),
        400
    );
    assert_eq!(
        t.admin(
            "PUT",
            &path,
            json!({"public_key":null}),
            Some(&t.token),
            true
        )
        .await
        .status(),
        204
    );
    assert!(
        t.state
            .tauri_integration
            .details(&t.app, "stable", false)
            .await
            .unwrap()
            .public_key
            .is_none()
    );
    t.state.sessions.logout(&t.actor).await.unwrap();
    assert!(matches!(
        t.state
            .applications
            .set_tauri_key(&t.actor, &t.app, Some(&key))
            .await,
        Err(veruna_releases::services::error::ServiceError::Unauthorized)
    ));
    assert_eq!(
        t.admin(
            "PUT",
            &path,
            json!({"public_key":key}),
            Some(&t.token),
            true
        )
        .await
        .status(),
        401
    );
}
#[tokio::test]
async fn tauri_integration_reuses_channel_readiness_and_preview_protocol() {
    let t = TestApp::new(DeliveryMode::Direct).await;
    t.state
        .applications
        .set_tauri_key(&t.actor, &t.app, Some(&public_key()))
        .await
        .unwrap();
    let r = t.create("1.0.1").await;
    let a = t.upload(&r, "windows-x86_64", "setup.exe").await;
    t.state
        .artifacts
        .set_updater(&t.actor, &t.app, &r.id, &a.id, Some("updater"), None)
        .await
        .unwrap();
    t.publish(&r, "stable").await;
    let d = t
        .state
        .tauri_integration
        .details(&t.app, "stable", false)
        .await
        .unwrap();
    assert!(d.blockers.iter().any(|i| i.code == "signature_missing"));
    assert!(!d.packages[0].signature_present);
    let preview = t
        .state
        .updater
        .preview(&t.app, "stable", "windows-x86_64", "1.0.0")
        .await
        .unwrap();
    assert_eq!(preview.reason, "signature_missing");
    assert_eq!(preview.state, "blocked");
    // Published files cannot be repaired: publish a new signed release instead.
    let signed = t.create("1.0.2").await;
    let signed_file = t.upload(&signed, "windows-x86_64", "setup.exe").await;
    t.state
        .artifacts
        .set_updater(
            &t.actor,
            &t.app,
            &signed.id,
            &signed_file.id,
            Some("updater"),
            Some(Some(signature().as_bytes())),
        )
        .await
        .unwrap();
    t.publish(&signed, "beta").await;
    let path = format!("/api/v1/admin/applications/{}/integrations/tauri", t.app);
    let details = json_body(
        t.admin(
            "POST",
            &format!("{path}/readiness?channel=beta"),
            Value::Null,
            Some(&t.token),
            true,
        )
        .await,
    )
    .await;
    assert_eq!(details["ready"], true);
    assert_eq!(details["full_integrity_checked"], true);
    assert_eq!(details["current_version"], "1.0.2");
    assert_eq!(details["packages"][0]["signature_present"], true);
    let preview_path = format!("/api/v1/admin/applications/{}/updater/tauri/preview", t.app);
    let available = json_body(
        t.admin(
            "POST",
            &preview_path,
            json!({"channel":"beta","target":"windows-x86_64","current_version":"1.0.0"}),
            Some(&t.token),
            true,
        )
        .await,
    )
    .await;
    assert_eq!(available["state"], "available");
    assert_eq!(available["artifact_filename"], "setup.exe");
    let actual = json_body(
        t.get(
            "GET",
            "/api/v1/updates/tauri/product/beta/windows/x86_64/1.0.0",
            &[],
        )
        .await,
    )
    .await;
    assert_eq!(available["response"], actual);
    let absent = json_body(
        t.admin(
            "POST",
            &preview_path,
            json!({"channel":"beta","target":"windows-x86_64","current_version":"1.0.2"}),
            Some(&t.token),
            true,
        )
        .await,
    )
    .await;
    assert_eq!(absent["reason"], "version_not_newer");
    assert!(absent["response"].is_null());
    let public = t
        .get(
            "GET",
            "/api/v1/updates/tauri/product/beta/windows/x86_64/1.0.2",
            &[],
        )
        .await;
    assert_eq!(public.status(), 204);
    assert!(bytes(public).await.is_empty());
    let absent = t
        .state
        .updater
        .preview(&t.app, "beta", "linux-aarch64", "1.0.0")
        .await
        .unwrap();
    assert_eq!(absent.reason, "updater_package_missing");
    assert!(
        t.state
            .updater
            .preview(&t.app, "beta", "android-aarch64", "1.0.0")
            .await
            .is_err()
    );
    let stable = t
        .state
        .tauri_integration
        .details(&t.app, "stable", false)
        .await
        .unwrap();
    assert_eq!(stable.current_version.as_deref(), Some("1.0.1"));
    tokio::fs::write(
        t.config.storage.directory.join(&signed_file.storage_key),
        b"XXXXXXXXXX",
    )
    .await
    .unwrap();
    let full = t
        .state
        .tauri_integration
        .details(&t.app, "beta", true)
        .await
        .unwrap();
    assert!(full.blockers.iter().any(|i| i.code == "checksum_mismatch"));
    t.state
        .applications
        .update(
            &t.actor,
            &t.app,
            ApplicationChanges {
                visibility: Some(Visibility::Private),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(
        t.state
            .tauri_integration
            .details(&t.app, "beta", false)
            .await
            .unwrap()
            .blockers
            .iter()
            .any(|i| i.code == "endpoint_unavailable")
    );
}
#[tokio::test]
async fn tauri_readiness_distinguishes_published_release_empty_channel_and_installer_only() {
    let t = TestApp::new(DeliveryMode::Direct).await;
    let r = t.create("1.0.1").await;
    t.upload(&r, "windows-x86_64", "setup.exe").await;
    t.publish(&r, "stable").await;
    let stable = t
        .state
        .tauri_integration
        .details(&t.app, "stable", false)
        .await
        .unwrap();
    assert!(stable.published_release_exists);
    assert!(
        stable
            .blockers
            .iter()
            .any(|i| i.code == "updater_package_missing")
    );
    let beta = t
        .state
        .tauri_integration
        .details(&t.app, "beta", false)
        .await
        .unwrap();
    assert!(beta.published_release_exists);
    assert!(
        beta.blockers
            .iter()
            .any(|i| i.code == "channel_release_missing")
    );
    assert!(
        !beta
            .blockers
            .iter()
            .any(|i| i.code == "published_release_missing")
    );
    let path = format!("/api/v1/admin/applications/{}/integrations/tauri", t.app);
    let cookie = format!("veruna_session={}", t.token);
    let settings = json_body(
        t.get(
            "GET",
            &path,
            &[
                ("cookie", &cookie),
                ("forwarded", "host=attacker.example;proto=http"),
            ],
        )
        .await,
    )
    .await;
    assert!(
        settings["endpoint"]
            .as_str()
            .unwrap()
            .starts_with("https://apps.example.test/")
    );
    assert_eq!(
        t.admin(
            "POST",
            &format!("{path}/readiness"),
            Value::Null,
            None,
            true
        )
        .await
        .status(),
        401
    );
    assert_eq!(
        t.admin(
            "POST",
            &format!("{path}/readiness"),
            Value::Null,
            Some(&t.token),
            false
        )
        .await
        .status(),
        403
    );
    t.state
        .users
        .create(
            &t.actor,
            "reader",
            Role::Operator,
            Some("test-password-123".into()),
        )
        .await
        .unwrap();
    let reader = t
        .state
        .authentication
        .login("reader", "test-password-123".into(), None)
        .await
        .unwrap()
        .token;
    assert_eq!(
        t.admin(
            "POST",
            &format!("{path}/readiness"),
            Value::Null,
            Some(&reader),
            true
        )
        .await
        .status(),
        200
    );
}
