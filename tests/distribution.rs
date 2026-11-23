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
        application::{ApplicationChanges, NewApplication, Status, Visibility},
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
        config.auth.public_origin = "http://127.0.0.1:8080".into();
        config.download.public_base_url = "http://127.0.0.1:8080".into();
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
                    distribution_role: None,
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
    async fn publish(&self, r: &Release) {
        let current = self.state.releases.get(&self.app, &r.id).await.unwrap();
        self.state
            .publishing
            .publish(
                &self.actor,
                &self.app,
                &r.id,
                PublicationCommand {
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
    fn url(&self, r: &Release, a: &Artifact) -> String {
        self.state.distribution.artifact_path("product", r, a)
    }
    async fn upload_type(&self, r: &Release, target: &str, name: &str, kind: &str) -> Value {
        let body = format!(
            "--FILE\r\nContent-Disposition: form-data; name=\"target\"\r\n\r\n{target}\r\n--FILE\r\nContent-Disposition: form-data; name=\"distribution_role\"\r\n\r\n{kind}\r\n--FILE\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{name}\"\r\n\r\n0123456789\r\n--FILE--\r\n"
        );
        let response = self
            .router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/api/v1/admin/applications/{}/releases/{}/artifacts",
                        self.app, r.id
                    ))
                    .header("origin", &self.config.auth.public_origin)
                    .header("x-veruna-request", "1")
                    .header("cookie", format!("veruna_session={}", self.token))
                    .header("x-csrf-token", secrets::csrf(&self.token))
                    .header("content-type", "multipart/form-data; boundary=FILE")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let value = json_body(response).await;
        assert_eq!(status, 201, "{value}");
        value
    }
}
async fn bytes(r: Response) -> Vec<u8> {
    r.into_body().collect().await.unwrap().to_bytes().to_vec()
}
async fn json_body(r: Response) -> Value {
    serde_json::from_slice(&bytes(r).await).unwrap()
}

#[tokio::test]
async fn universal_artifact_types_share_one_release_and_keep_original_bytes_without_keys() {
    use sha2::{Digest, Sha256};
    let t = TestApp::new(DeliveryMode::Direct).await;
    let r = t.create("0.5.0").await;
    let variants = [
        ("windows-x86_64", "mockport.exe", "executable", "exe"),
        ("linux-x86_64", "mockport", "executable", "bin"),
        ("linux-aarch64", "mockport-aarch64", "executable", "bin"),
        ("windows-x86_64", "MockPort-Setup.exe", "installer", "exe"),
        ("linux-x86_64", "mockport.tar.gz", "archive", "tar.gz"),
    ];
    let mut files = Vec::new();
    for (target, name, kind, format) in variants {
        let dto = t.upload_type(&r, target, name, kind).await;
        assert_eq!(dto["filename"], name);
        assert_eq!(dto["distribution_role"], kind);
        assert_eq!(dto["file_type"], format);
        assert_eq!(dto["signature_present"], false);
        assert_eq!(dto["standalone_signature_present"], false);
        assert_eq!(
            dto["sha256"],
            format!("{:x}", Sha256::digest(b"0123456789"))
        );
        files.push(dto);
    }
    t.publish(&r).await;
    let app = t.state.applications.get(&t.app).await.unwrap();
    assert!(
        t.state
            .tauri_integration
            .details(&t.app, false)
            .await
            .unwrap()
            .public_key
            .is_none()
    );
    assert!(app.standalone_public_key.is_none());
    assert!(app.product_id.is_none());
    for ((target, _, kind, format), dto) in variants.iter().zip(&files) {
        let path = format!(
            "/product/releases/0.5.0/artifacts/{}/download",
            dto["id"].as_str().unwrap()
        );
        let response = t.get("GET", &path, &[]).await;
        assert_eq!(response.status(), 200);
        assert!(
            response.headers()["content-disposition"]
                .to_str()
                .unwrap()
                .contains(dto["filename"].as_str().unwrap())
        );
        assert_eq!(bytes(response).await, b"0123456789");
        for method in ["GET", "HEAD"] {
            for latest in [
                format!("/product/latest?target={target}&type={kind}&format={format}"),
                format!("/product/latest/{target}?purpose={kind}&format={format}"),
                format!("/product/0.5.0/{target}?type={kind}&format={format}"),
            ] {
                let response = t.get(method, &latest, &[]).await;
                assert_eq!(response.status(), 307, "{latest}");
                assert_eq!(response.headers()["location"], path);
                assert_eq!(response.headers()["cache-control"], "no-store");
            }
        }
    }
    // Old target-only links keep selecting the installer even when a standalone EXE exists.
    assert!(
        t.get("GET", "/product/latest/windows-x86_64", &[])
            .await
            .headers()["location"]
            .to_str()
            .unwrap()
            .contains(files[3]["id"].as_str().unwrap())
    );
    for source in [
        "/product/manifest.json?limit=1",
        "/api/v1/public/applications/product",
    ] {
        let doc = json_body(t.get("GET", source, &[]).await).await;
        assert_eq!(
            doc["latest_by_type"]["executable"]["linux-x86_64"]["artifacts"][0]["purpose"],
            "executable"
        );
        assert_eq!(
            doc["latest_by_type"]["archive"]["linux-x86_64"]["version"],
            "0.5.0"
        );
        assert_eq!(
            doc["latest_by_target"]["windows-x86_64"]["artifacts"][0]["purpose"],
            "installer"
        );
    }
    let history = json_body(
        t.get("GET", "/api/v1/public/applications/product/releases", &[])
            .await,
    )
    .await;
    assert_eq!(
        history["items"][0]["artifacts"].as_array().unwrap().len(),
        5
    );
    // Adding an ordinary executable to a published release requires neither kind of signature.
    t.upload_type(&r, "darwin-aarch64", "mockport-macos", "executable")
        .await;
}

#[tokio::test]
async fn latest_filters_type_target_and_format_before_selecting_semver_and_reports_ambiguity() {
    let t = TestApp::new(DeliveryMode::Direct).await;
    let old = t.create("0.5.0").await;
    let executable = t
        .upload_type(&old, "linux-x86_64", "mockport", "executable")
        .await;
    let tar = t
        .upload_type(&old, "linux-x86_64", "mockport.tar.gz", "archive")
        .await;
    t.upload_type(&old, "linux-aarch64", "mockport-arm", "executable")
        .await;
    t.publish(&old).await;
    let new = t.create("0.10.0").await;
    t.upload_type(&new, "windows-x86_64", "setup.exe", "installer")
        .await;
    let zip = t
        .upload_type(&new, "linux-x86_64", "mockport.zip", "archive")
        .await;
    t.publish(&new).await;
    for (version, kind, format, file) in [
        ("0.5.0", "executable", "bin", &executable),
        ("0.5.0", "archive", "tar.gz", &tar),
        ("0.10.0", "archive", "zip", &zip),
    ] {
        let response = t
            .get(
                "GET",
                &format!("/product/latest/linux-x86_64?type={kind}&format={format}"),
                &[],
            )
            .await;
        assert_eq!(response.status(), 307);
        assert_eq!(
            response.headers()["location"],
            format!(
                "/product/releases/{version}/artifacts/{}/download",
                file["id"].as_str().unwrap()
            )
        );
    }
    t.upload_type(&new, "linux-x86_64", "mockport.tar.gz", "archive")
        .await;
    for query in ["type=executable", "target=linux-x86_64&type=archive"] {
        let response = t.get("GET", &format!("/product/latest?{query}"), &[]).await;
        assert_eq!(response.status(), 409);
        let error = json_body(response).await;
        assert_eq!(error["error"]["code"], "ambiguous_artifact");
        assert!(
            error["error"]["message"]
                .as_str()
                .unwrap()
                .contains("target, type (or purpose), and format")
        );
    }
    // An explicit unique type works without a target. Unknown or conflicting filters fail closed.
    assert_eq!(
        t.get("GET", "/product/latest?type=installer", &[])
            .await
            .status(),
        307
    );
    for (query, status) in [
        ("type=unknown", 422),
        ("type=executable&purpose=installer", 400),
        ("platform=linux", 400),
        ("target=linux-x86_64&type=installer", 404),
    ] {
        assert_eq!(
            t.get("GET", &format!("/product/latest?{query}"), &[])
                .await
                .status(),
            status
        );
    }
    let draft = t.create("8.0.0").await;
    t.upload_type(&draft, "linux-x86_64", "draft", "executable")
        .await;
    let pre = t.create("9.0.0-rc.1").await;
    t.upload_type(&pre, "linux-x86_64", "preview", "executable")
        .await;
    t.publish(&pre).await;
    let response = t
        .get(
            "GET",
            "/product/latest?target=linux-x86_64&type=executable",
            &[],
        )
        .await;
    assert!(
        response.headers()["location"]
            .to_str()
            .unwrap()
            .contains(executable["id"].as_str().unwrap())
    );
}

#[tokio::test]
async fn artifact_type_edits_keep_authorization_conflicts_and_published_immutability() {
    let t = TestApp::new(DeliveryMode::Direct).await;
    let r = t.create("0.5.0").await;
    let exe = t
        .upload_type(&r, "windows-x86_64", "app.exe", "executable")
        .await;
    let installer = t
        .upload_type(&r, "windows-x86_64", "setup.exe", "installer")
        .await;
    let path = format!(
        "/api/v1/admin/applications/{}/releases/{}/artifacts/{}/updater",
        t.app,
        r.id,
        installer["id"].as_str().unwrap()
    );
    let payload = json!({"distribution_role":"archive"});
    assert_eq!(
        t.admin("PUT", &path, payload.clone(), None, true)
            .await
            .status(),
        401
    );
    assert_eq!(
        t.admin("PUT", &path, payload.clone(), Some(&t.token), false)
            .await
            .status(),
        403
    );
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
        t.admin("PUT", &path, payload.clone(), Some(&operator), true)
            .await
            .status(),
        403
    );
    assert_eq!(
        t.admin(
            "PUT",
            &path,
            json!({"distribution_role":"executable"}),
            Some(&t.token),
            true
        )
        .await
        .status(),
        409
    );
    assert_eq!(
        t.admin(
            "PUT",
            &path,
            json!({"distribution_role":"unknown"}),
            Some(&t.token),
            true
        )
        .await
        .status(),
        422
    );
    let response = t
        .admin("PUT", &path, payload.clone(), Some(&t.token), true)
        .await;
    assert_eq!(response.status(), 200);
    assert_eq!(json_body(response).await["distribution_role"], "archive");
    t.publish(&r).await;
    assert_eq!(
        t.admin(
            "PUT",
            &path,
            json!({"distribution_role":"installer"}),
            Some(&t.token),
            true
        )
        .await
        .status(),
        409
    );
    // Database uniqueness and REPLACE protection apply to new types too.
    let pool = sqlx::SqlitePool::connect(&format!("sqlite://{}", t.config.database.path.display()))
        .await
        .unwrap();
    let a = t
        .state
        .artifacts
        .list(&t.app, &r.id)
        .await
        .unwrap()
        .into_iter()
        .find(|a| a.id == exe["id"].as_str().unwrap())
        .unwrap();
    let result = sqlx::query("INSERT OR REPLACE INTO artifacts SELECT ?,release_id,filename,target,file_type,size_bytes,sha256,?,created_at,distribution_role,updater_signature,standalone_signature FROM artifacts WHERE id=?")
        .bind(uuid::Uuid::new_v4().to_string()).bind(format!("{}.blob", uuid::Uuid::new_v4().simple())).bind(&a.id).execute(&pool).await;
    assert!(result.is_err());
    assert_eq!(
        t.state.artifacts.list(&t.app, &r.id).await.unwrap().len(),
        2
    );
}

#[tokio::test]
async fn public_applications_catalog_requires_public_active_and_published() {
    let mut t = TestApp::new(DeliveryMode::Direct).await;
    let path = "/api/v1/public/applications";
    assert_eq!(json_body(t.get("GET", path, &[]).await).await["total"], 0);
    let draft = t.create("1.0.0").await;
    t.upload(&draft, "windows-x86_64", "setup.exe").await;
    assert_eq!(json_body(t.get("GET", path, &[]).await).await["total"], 0);
    t.publish(&draft).await;
    // Multiple published versions still produce one application.
    let second = t.create("2.0.0-rc.1").await;
    t.upload(&second, "windows-x86_64", "setup.exe").await;
    t.publish(&second).await;
    let response = t
        .get("GET", path, &[("origin", "https://client.example")])
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(response.headers()["access-control-allow-origin"], "*");
    assert_eq!(
        json_body(response).await,
        json!({
            "items": [{"name": "Приложение", "slug": "product", "icon_url": null}],
            "total": 1, "limit": 20, "offset": 0,
        })
    );
    let original = t.app.clone();
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgba8(2, 2)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    t.state
        .applications
        .set_icon(&t.actor, &original, Some(png.into_inner()))
        .await
        .unwrap();
    let with_icon = json_body(t.get("GET", path, &[]).await).await;
    let icon = with_icon["items"][0]["icon_url"].as_str().unwrap();
    let response = t.get("GET", icon, &[]).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "image/png");
    // A second app with only a published prerelease belongs in the catalog too.
    t.app = t
        .state
        .applications
        .create(
            &t.actor,
            NewApplication {
                slug: "alpha".into(),
                name: "Alpha".into(),
                description: String::new(),
                visibility: Visibility::Public,
            },
        )
        .await
        .unwrap()
        .id;
    let preview = t.create("0.1.0-beta.1").await;
    t.upload(&preview, "windows-x86_64", "preview.exe").await;
    t.publish(&preview).await;
    let first = json_body(t.get("GET", &format!("{path}?limit=1"), &[]).await).await;
    assert_eq!(first["total"], 2);
    assert_eq!(first["items"][0]["slug"], "alpha");
    let next = json_body(t.get("GET", &format!("{path}?limit=1&offset=1"), &[]).await).await;
    assert_eq!(next["items"].as_array().unwrap().len(), 1);
    assert_eq!(next["items"][0]["slug"], "product");
    assert_eq!(next["offset"], 1);
    t.state
        .applications
        .update(
            &t.actor,
            &original,
            ApplicationChanges {
                visibility: Some(Visibility::Private),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let hidden = json_body(t.get("GET", path, &[]).await).await;
    assert_eq!(hidden["total"], 1);
    assert_eq!(hidden["items"][0]["slug"], "alpha");
    t.state
        .applications
        .update(
            &t.actor,
            &original,
            ApplicationChanges {
                visibility: Some(Visibility::Public),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    t.state
        .applications
        .set_status(&t.actor, &original, Status::Archived)
        .await
        .unwrap();
    assert_eq!(json_body(t.get("GET", path, &[]).await).await["total"], 1);
    let head = t.get("HEAD", path, &[]).await;
    assert_eq!(head.status(), StatusCode::OK);
    assert!(bytes(head).await.is_empty());
}

#[tokio::test]
async fn public_applications_catalog_validates_pagination_and_methods() {
    let t = TestApp::new(DeliveryMode::Direct).await;
    for query in [
        "limit=0",
        "limit=101",
        "offset=100001",
        "offset=-1",
        "limit=text",
        "visibility=private",
    ] {
        assert_eq!(
            t.get("GET", &format!("/api/v1/public/applications?{query}"), &[])
                .await
                .status(),
            StatusCode::BAD_REQUEST,
            "{query}"
        );
    }
    assert_eq!(
        t.get("POST", "/api/v1/public/applications", &[])
            .await
            .status(),
        StatusCode::METHOD_NOT_ALLOWED
    );
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

const CORS_ORIGINS: [&str; 2] = ["https://videolov.ru", "https://factum.systems"];
const CORS_PATHS: [&str; 5] = [
    "/product/manifest.json",
    "/api/v1/public/applications/product",
    "/api/v1/public/applications/product/releases",
    "/api/v1/public/applications/product/icon",
    "/api/v1/updates/tauri/product/windows/x86_64/0.0.0",
];

fn assert_public_cors(response: &Response) {
    assert_eq!(response.headers()["access-control-allow-origin"], "*");
    assert_eq!(response.headers()["access-control-expose-headers"], "etag");
    assert!(
        !response
            .headers()
            .contains_key("access-control-allow-credentials")
    );
    assert!(!response.headers().contains_key("set-cookie"));
}

fn assert_no_cors(response: &Response) {
    assert!(
        response
            .headers()
            .keys()
            .all(|name| !name.as_str().starts_with("access-control-"))
    );
}

#[tokio::test]
async fn public_cors_preserves_get_head_manifest_etag_and_updater_no_content() {
    let t = TestApp::new(DeliveryMode::Direct).await;
    let r = t.create("1.0.0").await;
    let a = t.upload(&r, "windows-x86_64", "setup.exe").await;
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
    let mut icon = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgba8(2, 2)
        .write_to(&mut icon, image::ImageFormat::Png)
        .unwrap();
    t.state
        .applications
        .set_icon(&t.actor, &t.app, Some(icon.into_inner()))
        .await
        .unwrap();
    t.publish(&r).await;

    for path in CORS_PATHS
        .into_iter()
        .chain(["/api/v1/updates/tauri/product/windows/x86_64/1.0.0"])
    {
        let baseline = t.get("GET", path, &[]).await;
        let status = baseline.status();
        assert_eq!(status, if path.ends_with("/1.0.0") { 204 } else { 200 });
        let headers = baseline.headers().clone();
        assert_eq!(
            headers["cache-control"],
            if path.ends_with("manifest.json") {
                "public, no-cache"
            } else {
                "no-store"
            }
        );
        if status == StatusCode::OK {
            assert_eq!(
                headers["content-type"],
                if path.ends_with("/icon") {
                    "image/png"
                } else {
                    "application/json"
                }
            );
        }
        let body = bytes(baseline).await;
        for origin in CORS_ORIGINS {
            for method in ["GET", "HEAD"] {
                let response = t.get(method, path, &[("origin", origin)]).await;
                assert_eq!(response.status(), status, "{method} {path}");
                assert_public_cors(&response);
                for name in ["etag", "cache-control", "content-type"] {
                    assert_eq!(response.headers().get(name), headers.get(name), "{path}");
                }
                if method == "HEAD" {
                    assert!(bytes(response).await.is_empty());
                } else {
                    assert_eq!(bytes(response).await, body);
                }
            }
        }
    }
    let manifest = t.get("GET", CORS_PATHS[0], &[]).await;
    assert_eq!(manifest.headers()["content-type"], "application/json");
    assert_eq!(manifest.headers()["cache-control"], "public, no-cache");
    let etag = manifest.headers()["etag"].to_str().unwrap();
    for origin in CORS_ORIGINS {
        for method in ["GET", "HEAD"] {
            let response = t
                .get(
                    method,
                    CORS_PATHS[0],
                    &[("origin", origin), ("if-none-match", etag)],
                )
                .await;
            assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
            assert_public_cors(&response);
            assert_eq!(response.headers()["etag"], etag);
            assert_eq!(response.headers()["cache-control"], "public, no-cache");
            assert!(bytes(response).await.is_empty());
        }
    }
}

#[tokio::test]
async fn public_cors_preflight_allows_only_read_methods_and_conditional_header() {
    let t = TestApp::new(DeliveryMode::Direct).await;
    for origin in CORS_ORIGINS {
        for path in CORS_PATHS {
            for method in ["GET", "HEAD", "POST", "PUT", "DELETE"] {
                let response = t
                    .get(
                        "OPTIONS",
                        path,
                        &[
                            ("origin", origin),
                            ("access-control-request-method", method),
                            ("access-control-request-headers", "if-none-match"),
                        ],
                    )
                    .await;
                assert!(response.status().is_success(), "{path}");
                assert_eq!(response.headers()["access-control-allow-origin"], "*");
                assert_eq!(
                    response.headers()["access-control-allow-methods"],
                    "GET,HEAD"
                );
                assert_eq!(
                    response.headers()["access-control-allow-headers"],
                    "if-none-match"
                );
                assert!(
                    !response
                        .headers()
                        .contains_key("access-control-allow-credentials")
                );
                assert!(bytes(response).await.is_empty());
            }
            for method in ["POST", "PUT", "DELETE"] {
                let response = t.get(method, path, &[("origin", origin)]).await;
                assert_eq!(response.status(), 405);
                assert_public_cors(&response);
            }
        }
    }
}

#[tokio::test]
async fn public_cors_covers_errors_without_bypassing_visibility_or_storage_checks() {
    let t = TestApp::new(DeliveryMode::Direct).await;
    let r = t.create("1.0.0").await;
    let a = t.upload(&r, "windows-x86_64", "setup.exe").await;
    t.publish(&r).await;
    for origin in CORS_ORIGINS {
        for path in CORS_PATHS {
            let response = t
                .get(
                    "GET",
                    &path.replace("product", "missing"),
                    &[("origin", origin)],
                )
                .await;
            assert_eq!(response.status(), 404, "{path}");
            assert_public_cors(&response);
        }
        for (path, status) in [
            ("/product/manifest.json?unknown=1", 400),
            ("/api/v1/public/applications/product?unknown=1", 400),
            ("/api/v1/public/applications/product/releases?limit=0", 400),
            ("/api/v1/updates/tauri/product/windows/x86_64/invalid", 422),
        ] {
            let response = t.get("GET", path, &[("origin", origin)]).await;
            assert_eq!(response.status(), status, "{path}");
            assert_public_cors(&response);
            assert_eq!(response.headers()["cache-control"], "no-store");
            assert_eq!(response.headers()["content-type"], "application/json");
        }
    }
    tokio::fs::remove_file(t.config.storage.directory.join(&a.storage_key))
        .await
        .unwrap();
    for origin in CORS_ORIGINS {
        for path in &CORS_PATHS[..3] {
            let response = t.get("GET", path, &[("origin", origin)]).await;
            assert_eq!(response.status(), 503, "{path}");
            assert_public_cors(&response);
        }
    }
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
        for origin in CORS_ORIGINS {
            for path in CORS_PATHS {
                let response = t
                    .get("GET", path, &[("origin", origin), ("if-none-match", "*")])
                    .await;
                assert_eq!(response.status(), 404, "{path}");
                assert_public_cors(&response);
            }
        }
    }
}

#[tokio::test]
async fn cors_is_absent_from_admin_auth_downloads_and_unmatched_routes() {
    let t = TestApp::new(DeliveryMode::Direct).await;
    let r = t.create("1.0.0").await;
    let a = t.upload(&r, "windows-x86_64", "setup.exe").await;
    t.publish(&r).await;
    let download = t.url(&r, &a);
    let cookie = format!("veruna_session={}", t.token);
    for origin in CORS_ORIGINS {
        for (path, anonymous_status, authenticated_status) in [
            ("/api/v1/admin/applications", 401, 200),
            ("/api/v1/admin/missing", 401, 404),
            ("/api/v1/auth/me", 401, 200),
            ("/api/v1/health", 200, 200),
            ("/api/v1/public/applications/product/missing", 404, 404),
            ("/api/v1/updates/tauri/missing", 404, 404),
            ("/product", 200, 200),
            ("/product/latest", 307, 307),
            (download.as_str(), 200, 200),
        ] {
            for (headers, status) in [
                (vec![("origin", origin)], anonymous_status),
                (
                    vec![("origin", origin), ("cookie", cookie.as_str())],
                    authenticated_status,
                ),
            ] {
                let response = t.get("GET", path, &headers).await;
                assert_eq!(response.status(), status, "{path}");
                assert_no_cors(&response);
            }
            let response = t
                .get(
                    "OPTIONS",
                    path,
                    &[("origin", origin), ("access-control-request-method", "GET")],
                )
                .await;
            assert!(!response.status().is_success(), "{path}");
            assert_no_cors(&response);
        }
        for (path, headers) in [
            (
                "/api/v1/auth/login",
                vec![("origin", origin), ("x-veruna-request", "1")],
            ),
            (
                "/api/v1/auth/logout",
                vec![
                    ("origin", origin),
                    ("cookie", cookie.as_str()),
                    ("x-veruna-request", "1"),
                    ("x-csrf-token", &secrets::csrf(&t.token)),
                ],
            ),
        ] {
            let response = t.get("POST", path, &headers).await;
            assert_eq!(response.status(), 403);
            assert_no_cors(&response);
        }
    }
}

#[tokio::test]
async fn public_cors_also_wraps_automatic_origin_resolution_errors() {
    let mut t = TestApp::new(DeliveryMode::Direct).await;
    t.state.auth_config.public_origin.clear();
    t.state.distribution.config.public_base_url.clear();
    t.router = http::router(t.state.clone());
    // TestApp::get uses an untrusted public Host over HTTP, rejected in auto mode.
    for origin in CORS_ORIGINS {
        for path in CORS_PATHS {
            let response = t.get("GET", path, &[("origin", origin)]).await;
            assert_eq!(response.status(), 400, "{path}");
            assert_public_cors(&response);
        }
    }
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
    t.publish(&r).await;
    assert_eq!(
        t.get("GET", "/product/latest/windows-x86_64", &[])
            .await
            .headers()["location"],
        t.url(&r, &m)
    );
    assert_eq!(
        t.get("GET", "/product/1.0.0/windows-x86_64?purpose=updater", &[])
            .await
            .headers()["location"],
        t.url(&r, &a)
    );
    assert_eq!(
        t.get("GET", "/product/latest/windows-x86_64?format=exe", &[])
            .await
            .status(),
        404
    );
    let r2 = t.create("1.1.0").await;
    t.upload(&r2, "windows-x86_64", "setup.exe").await;
    t.upload(&r2, "windows-x86_64", "setup.msi").await;
    t.publish(&r2).await;
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
    t.publish(&r).await;
    let url = t.url(&r, &a);
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
    let url = t.url(&r, &a);
    assert_eq!(t.get("GET", &url, &[]).await.status(), 404);
    let empty = json_body(
        t.get("GET", "/api/v1/public/applications/product", &[])
            .await,
    )
    .await;
    assert!(empty["latest_by_target"].as_object().unwrap().is_empty());
    assert_eq!(
        empty["manifest_url"],
        "https://apps.example.test/product/manifest.json"
    );
    t.publish(&r).await;
    let catalog = json_body(
        t.get("GET", "/api/v1/public/applications/product", &[])
            .await,
    )
    .await;
    assert_eq!(
        catalog["latest_by_target"]["windows-x86_64"]["version"],
        "1.0.0"
    );
    assert_eq!(
        catalog["latest_by_target"]["windows-x86_64"]["artifacts"][0]["download_url"],
        format!("https://apps.example.test{url}")
    );
    let manifest = json_body(t.get("GET", "/product/manifest.json", &[]).await).await;
    assert_eq!(
        manifest["releases"][0]["artifacts"][0]["download_url"],
        format!("https://apps.example.test{url}")
    );
    assert!(!catalog.to_string().contains("storage_key"));
    assert!(!catalog.to_string().contains("created_at"));
    let b = t.create("2.0.0-rc.1").await;
    t.upload(&b, "linux-x86_64", "prerelease.AppImage").await;
    t.publish(&b).await;
    let history = json_body(
        t.get(
            "GET",
            "/api/v1/public/applications/product/releases?limit=1&offset=1",
            &[],
        )
        .await,
    )
    .await;
    assert_eq!(history["total"], 2);
    assert_eq!(history["items"][0]["version"], "1.0.0");
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
            "/api/v1/public/applications/product/releases?unknown=no",
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
            "/api/v1/updates/tauri/product/windows/x86_64/0.0.0",
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
    t.publish(&r).await;
    let path = t.config.storage.directory.join(&a.storage_key);
    tokio::fs::remove_file(&path).await.unwrap();
    let response = t.get("GET", &t.url(&r, &a), &[]).await;
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
        assert_eq!(t.get("HEAD", &t.url(&r, &a), &[]).await.status(), 503);
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
    t.publish(&r).await;
    let response = t
        .get("GET", &t.url(&r, &a), &[("range", "bytes=1-3")])
        .await;
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.headers()["x-accel-redirect"],
        format!("/_veruna_files/{}", a.storage_key)
    );
    assert!(bytes(response).await.is_empty());
    let head = t.get("HEAD", &t.url(&r, &a), &[]).await;
    assert_eq!(head.headers()["content-length"], "10");
    assert!(!head.headers().contains_key("x-accel-redirect"));
    assert!(bytes(head).await.is_empty());
    let invalid = t
        .get("GET", &t.url(&r, &a), &[("range", "bytes=100-")])
        .await;
    assert_eq!(invalid.status(), 416);
    assert!(!invalid.headers().contains_key("x-accel-redirect"));
    t.state
        .applications
        .set_status(&t.actor, &t.app, Status::Archived)
        .await
        .unwrap();
    assert_eq!(t.get("GET", &t.url(&r, &a), &[]).await.status(), 404);
}
#[tokio::test]
async fn macos_universal_download_and_updater_serve_both_architectures() {
    let t = TestApp::new(DeliveryMode::Direct).await;
    let r = t.create("1.2.0").await;
    let installer = t.upload(&r, "darwin-universal", "product.dmg").await;
    let updater = t.upload(&r, "darwin-universal", "product.app.tar.gz").await;
    let sig = signature();
    t.state
        .artifacts
        .set_updater(
            &t.actor,
            &t.app,
            &r.id,
            &updater.id,
            Some("updater"),
            Some(Some(sig.as_bytes())),
        )
        .await
        .unwrap();
    t.publish(&r).await;
    let latest = t.get("GET", "/product/latest/darwin-universal", &[]).await;
    assert_eq!(latest.status(), 307);
    assert_eq!(
        latest.headers()["location"],
        format!(
            "/product/releases/{}/artifacts/{}/download",
            r.version, installer.id
        )
    );
    for arch in ["x86_64", "aarch64", "universal"] {
        let response = t
            .get(
                "GET",
                &format!("/api/v1/updates/tauri/product/darwin/{arch}/1.0.0"),
                &[],
            )
            .await;
        assert_eq!(response.status(), 200);
        let data = json_body(response).await;
        assert_eq!(data["signature"], sig);
        assert_eq!(
            data["url"],
            format!("https://apps.example.test{}", t.url(&r, &updater))
        );
        let preview = t
            .state
            .updater
            .preview(&t.app, &format!("darwin-{arch}"), "1.0.0")
            .await
            .unwrap();
        assert_eq!(preview.state, "available");
        assert_eq!(preview.artifact_id.as_deref(), Some(updater.id.as_str()));
    }
    assert_eq!(
        bytes(t.get("GET", &t.url(&r, &updater), &[]).await).await,
        b"0123456789"
    );
}
#[tokio::test]
async fn tauri_http_updates_signatures_and_no_downgrades() {
    let t = TestApp::new(DeliveryMode::Direct).await;
    let endpoint = "/api/v1/updates/tauri/product/windows/x86_64/1.0.0";
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
    t.publish(&r).await;
    let response = t.get("GET", endpoint, &[]).await;
    assert_eq!(response.status(), 200);
    let data = json_body(response).await;
    assert_eq!(data["signature"], sig);
    assert_eq!(data["version"], "1.2.0");
    assert_eq!(
        data["url"],
        format!("https://apps.example.test{}", t.url(&r, &a))
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
    let prerelease = t.create("1.3.0-rc.1").await;
    let b = t
        .upload(&prerelease, "windows-x86_64", "prerelease.exe")
        .await;
    t.state
        .artifacts
        .set_updater(
            &t.actor,
            &t.app,
            &prerelease.id,
            &b.id,
            Some("updater"),
            Some(Some(sig.as_bytes())),
        )
        .await
        .unwrap();
    t.publish(&prerelease).await;
    assert_eq!(
        json_body(t.get("GET", endpoint, &[]).await).await["version"],
        "1.2.0"
    );
    let unsigned = t.create("1.4.0").await;
    let u = t.upload(&unsigned, "windows-x86_64", "unsigned.exe").await;
    t.state
        .artifacts
        .set_updater(&t.actor, &t.app, &unsigned.id, &u.id, Some("updater"), None)
        .await
        .unwrap();
    t.publish(&unsigned).await;
    assert_eq!(
        json_body(t.get("GET", endpoint, &[]).await).await["version"],
        "1.2.0"
    );
    assert_eq!(
        t.get("GET", "/product/latest/windows-x86_64", &[])
            .await
            .status(),
        404
    );
    let preview = t
        .state
        .updater
        .preview(&t.app, "windows-x86_64", "1.0.0")
        .await
        .unwrap();
    assert_eq!(preview.selected_version.as_deref(), Some("1.2.0"));
    t.publish(&r).await;
    assert_eq!(
        t.get("GET", &endpoint.replace("/1.0.0", "/1.4.0"), &[])
            .await
            .status(),
        204
    );
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
    veruna_releases::repositories::release::ReleasesRepository::publish(
        &mut tx,
        &r.id,
        secrets::now(),
    )
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
    let generated = endpoint("https://updates.example.test/", "product-one").unwrap();
    assert_eq!(
        generated,
        "https://updates.example.test/api/v1/updates/tauri/product-one/{{target}}/{{arch}}/{{current_version}}"
    );
    let prerelease = endpoint("https://another.example.test", "second-app").unwrap();
    assert!(
        prerelease.starts_with("https://another.example.test/api/v1/updates/tauri/second-app/")
    );
    let key = "quoted\"key\nwith\\escapes";
    let value: Value = serde_json::from_str(&configuration(key, &generated).to_string()).unwrap();
    assert_eq!(value["plugins"]["updater"]["pubkey"], key);
    assert_eq!(value["plugins"]["updater"]["endpoints"][0], generated);
    assert_eq!(value["bundle"]["createUpdaterArtifacts"], true);
    for base in [
        "https://evil.test/path",
        "https://user:password@example.test",
        "https://example.test?x=1",
        "file:///tmp",
    ] {
        assert!(endpoint(base, "product").is_err());
    }
    assert!(endpoint("https://example.test", "../bad").is_err());
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
    for code in ["public_key_missing", "published_release_missing"] {
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
        .details(&t.app, false)
        .await
        .unwrap();
    assert_eq!(d.public_key.as_deref(), Some(key.as_str()));
    assert!(
        d.endpoint
            .contains("https://apps.example.test/api/v1/updates/tauri/product/{{target}}")
    );
    assert_eq!(
        d.configuration.unwrap()["plugins"]["updater"]["pubkey"],
        key
    );
    assert_eq!(
        t.admin(
            "GET",
            &format!("{path}?unknown=bad"),
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
            .details(&t.app, false)
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
async fn tauri_integration_reuses_readiness_and_preview_protocol() {
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
    t.publish(&r).await;
    let d = t
        .state
        .tauri_integration
        .details(&t.app, false)
        .await
        .unwrap();
    assert!(
        d.blockers
            .iter()
            .any(|i| i.code == "signed_updater_missing")
    );
    assert!(d.packages.is_empty());
    let preview = t
        .state
        .updater
        .preview(&t.app, "windows-x86_64", "1.0.0")
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
    t.publish(&signed).await;
    let path = format!("/api/v1/admin/applications/{}/integrations/tauri", t.app);
    let details = json_body(
        t.admin(
            "POST",
            &format!("{path}/readiness"),
            Value::Null,
            Some(&t.token),
            true,
        )
        .await,
    )
    .await;
    assert_eq!(details["ready"], true);
    assert_eq!(details["full_integrity_checked"], true);
    assert_eq!(details["packages"][0]["version"], "1.0.2");
    assert_eq!(details["packages"][0]["signature_present"], true);
    let preview_path = format!("/api/v1/admin/applications/{}/updater/tauri/preview", t.app);
    let available = json_body(
        t.admin(
            "POST",
            &preview_path,
            json!({"target":"windows-x86_64","current_version":"1.0.0"}),
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
            "/api/v1/updates/tauri/product/windows/x86_64/1.0.0",
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
            json!({"target":"windows-x86_64","current_version":"1.0.2"}),
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
            "/api/v1/updates/tauri/product/windows/x86_64/1.0.2",
            &[],
        )
        .await;
    assert_eq!(public.status(), 204);
    assert!(bytes(public).await.is_empty());
    let absent = t
        .state
        .updater
        .preview(&t.app, "linux-aarch64", "1.0.0")
        .await
        .unwrap();
    assert_eq!(absent.reason, "updater_package_missing");
    assert!(
        t.state
            .updater
            .preview(&t.app, "android-aarch64", "1.0.0")
            .await
            .is_err()
    );
    let integration = t
        .state
        .tauri_integration
        .details(&t.app, false)
        .await
        .unwrap();
    assert_eq!(
        integration.packages.first().map(|p| p.version.as_str()),
        Some("1.0.2")
    );
    tokio::fs::write(
        t.config.storage.directory.join(&signed_file.storage_key),
        b"XXXXXXXXXX",
    )
    .await
    .unwrap();
    let full = t
        .state
        .tauri_integration
        .details(&t.app, true)
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
            .details(&t.app, false)
            .await
            .unwrap()
            .blockers
            .iter()
            .any(|i| i.code == "endpoint_unavailable")
    );
}
#[tokio::test]
async fn tauri_readiness_distinguishes_published_release_and_installer_only() {
    let t = TestApp::new(DeliveryMode::Direct).await;
    let r = t.create("1.0.1").await;
    t.upload(&r, "windows-x86_64", "setup.exe").await;
    t.publish(&r).await;
    let integration = t
        .state
        .tauri_integration
        .details(&t.app, false)
        .await
        .unwrap();
    assert!(integration.published_release_exists);
    assert!(
        integration
            .blockers
            .iter()
            .any(|i| i.code == "signed_updater_missing")
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

#[tokio::test]
async fn automatic_domain_is_used_in_manifests_tauri_and_integration_without_shared_state() {
    use axum::extract::ConnectInfo;
    let mut t = TestApp::new(DeliveryMode::Direct).await;
    let r = t.create("1.0.0").await;
    let a = t.upload(&r, "windows-x86_64", "setup.exe").await;
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
    t.publish(&r).await;
    t.state.auth_config.public_origin.clear();
    t.state.distribution.config.public_base_url.clear();
    t.router = http::router(t.state.clone());
    for host in ["releases.example.test", "another.example.test:8443"] {
        let origin = format!("https://{host}");
        for path in [
            "/product/manifest.json".to_owned(),
            "/api/v1/public/applications/product".to_owned(),
            "/api/v1/updates/tauri/product/windows/x86_64/0.1.0".to_owned(),
            format!("/api/v1/admin/applications/{}/integrations/tauri", t.app),
        ] {
            let response = t
                .router
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(&path)
                        .extension(ConnectInfo(
                            "127.0.0.1:12345".parse::<std::net::SocketAddr>().unwrap(),
                        ))
                        .header("host", host)
                        .header("x-forwarded-proto", "https")
                        .header("x-forwarded-host", "attacker.example")
                        .header("cookie", format!("__Host-veruna_session={}", t.token))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), 200, "{path}");
            let value = json_body(response).await;
            if path.ends_with("manifest.json") {
                assert_eq!(
                    value["releases"][0]["artifacts"][0]["download_url"],
                    format!("{origin}{}", t.url(&r, &a))
                );
            } else if path == "/api/v1/public/applications/product" {
                assert_eq!(
                    value["manifest_url"],
                    format!("{origin}/product/manifest.json")
                );
            } else if path.contains("/updates/tauri/") {
                assert_eq!(value["url"], format!("{origin}{}", t.url(&r, &a)));
            } else {
                assert_eq!(value["public_base_url"], origin);
                assert!(
                    value["endpoint"]
                        .as_str()
                        .unwrap()
                        .starts_with(&format!("{origin}/api/v1/updates/tauri/"))
                );
            }
        }
    }
    assert!(t.state.distribution.config.public_base_url.is_empty());
}

#[tokio::test]
async fn latest_installers_are_independent_per_target_and_format_with_semver_and_draft_exclusion() {
    let t = TestApp::new(DeliveryMode::Direct).await;
    let empty = json_body(t.get("GET", "/product/manifest.json", &[]).await).await;
    assert_eq!(empty["schema_version"], 2);
    assert_eq!(empty["releases"], json!([]));
    assert_eq!(empty["latest_by_target"], json!({}));
    let page = t.get("GET", "/product/latest", &[]).await;
    assert_eq!(page.status(), 307);
    assert_eq!(page.headers()["location"], "/product");
    assert_eq!(page.headers()["cache-control"], "no-store");
    let r5 = t.create("0.5.0").await;
    let mac5 = t.upload(&r5, "darwin-aarch64", "product.dmg").await;
    t.publish(&r5).await;
    let original = t.state.releases.get(&t.app, &r5.id).await.unwrap();
    let win5 = t.upload(&r5, "windows-x86_64", "product.exe").await;
    let appended = t.state.releases.get(&t.app, &r5.id).await.unwrap();
    assert_eq!(original.published_at, appended.published_at);
    assert_eq!(appended.revision, original.revision + 1);
    let r6 = t.create("0.6.0").await;
    let mac6 = t.upload(&r6, "darwin-aarch64", "new.dmg").await;
    t.publish(&r6).await;
    let draft = t.create("9.0.0").await;
    t.upload(&draft, "windows-x86_64", "draft.exe").await;
    let pre = t.create("10.0.0-rc.1").await;
    let pre_file = t.upload(&pre, "windows-x86_64", "preview.exe").await;
    t.publish(&pre).await;
    for (target, r, file) in [
        ("windows-x86_64", &r5, &win5),
        ("darwin-aarch64", &r6, &mac6),
    ] {
        for method in ["GET", "HEAD"] {
            let latest = t
                .get(method, &format!("/product/latest/{target}"), &[])
                .await;
            assert_eq!(latest.status(), 307);
            assert_eq!(latest.headers()["location"], t.url(r, file));
        }
    }
    let manifest = json_body(t.get("GET", "/product/manifest.json?limit=1", &[]).await).await;
    assert_eq!(
        manifest["latest_by_target"]["windows-x86_64"]["version"],
        "0.5.0"
    );
    assert_eq!(
        manifest["latest_by_target"]["darwin-aarch64"]["version"],
        "0.6.0"
    );
    assert_eq!(manifest["releases"].as_array().unwrap().len(), 1);
    assert!(manifest.get("latest").is_none());
    // New formats can be appended to the same published platform.
    let msi5 = t.upload(&r5, "windows-x86_64", "product.msi").await;
    let ambiguous = t.get("GET", "/product/latest/windows-x86_64", &[]).await;
    assert_eq!(ambiguous.status(), 409);
    assert_eq!(
        json_body(ambiguous).await["error"]["code"],
        "ambiguous_artifact"
    );
    for (format, file) in [("exe", &win5), ("msi", &msi5)] {
        assert_eq!(
            t.get(
                "GET",
                &format!("/product/latest/windows-x86_64?format={format}"),
                &[]
            )
            .await
            .headers()["location"],
            t.url(&r5, file)
        );
    }
    // SemVer 0.10.0 > 0.6.0, while the older MSI remains available by format.
    let r10 = t.create("0.10.0+build.1").await;
    let win10 = t.upload(&r10, "windows-x86_64", "next.exe").await;
    t.publish(&r10).await;
    assert_eq!(
        t.get("GET", "/product/latest/windows-x86_64", &[])
            .await
            .headers()["location"],
        t.url(&r10, &win10)
    );
    assert_eq!(
        t.get("GET", "/product/latest/windows-x86_64?format=msi", &[])
            .await
            .headers()["location"],
        t.url(&r5, &msi5)
    );
    for (r, file) in [(&r5, &win5), (&r5, &mac5), (&r5, &msi5), (&pre, &pre_file)] {
        assert_eq!(t.get("GET", &t.url(r, file), &[]).await.status(), 200);
    }
    assert_eq!(
        t.get("GET", "/product/9.0.0/windows-x86_64", &[])
            .await
            .status(),
        404
    );
}

#[tokio::test]
async fn updater_finds_older_signed_package_when_newer_release_has_only_another_platform() {
    let t = TestApp::new(DeliveryMode::Direct).await;
    let win = t.create("0.5.0").await;
    let artifact = t.upload(&win, "windows-x86_64", "update.exe").await;
    t.state
        .artifacts
        .set_updater(
            &t.actor,
            &t.app,
            &win.id,
            &artifact.id,
            Some("updater"),
            Some(Some(signature().as_bytes())),
        )
        .await
        .unwrap();
    t.publish(&win).await;
    let mac = t.create("0.6.0").await;
    t.upload(&mac, "darwin-aarch64", "product.dmg").await;
    t.upload_type(&mac, "windows-x86_64", "product.exe", "executable")
        .await;
    t.publish(&mac).await;
    let response = json_body(
        t.get(
            "GET",
            "/api/v1/updates/tauri/product/windows/x86_64/0.4.0",
            &[],
        )
        .await,
    )
    .await;
    assert_eq!(response["version"], "0.5.0");
    assert!(response["url"].as_str().unwrap().contains(&artifact.id));
    assert_eq!(
        t.get(
            "GET",
            "/api/v1/updates/tauri/product/windows/x86_64/0.5.0",
            &[]
        )
        .await
        .status(),
        204
    );
}

#[tokio::test]
async fn download_counts_track_starts_with_identical_direct_and_nginx_semantics() {
    for mode in [DeliveryMode::Direct, DeliveryMode::Nginx] {
        let t = TestApp::new(mode).await;
        let r = t.create("1.0.0").await;
        let a = t.upload(&r, "windows-x86_64", "setup.exe").await;
        t.publish(&r).await;
        let url = t.url(&r, &a);
        let head = t.get("HEAD", &url, &[]).await;
        assert_eq!(head.status(), 200);
        let etag = head.headers()["etag"].to_str().unwrap().to_owned();
        drop(head);
        for (headers, status) in [
            (vec![("if-none-match", etag.as_str())], 304),
            (vec![("if-match", "\"obsolete\"")], 412),
            (vec![("range", "bytes=999-")], 416),
            (vec![("range", "bytes=invalid")], 416),
            (
                vec![("range", "bytes=2-5")],
                if mode == DeliveryMode::Direct {
                    206
                } else {
                    200
                },
            ),
            (
                vec![("range", "bytes=-3")],
                if mode == DeliveryMode::Direct {
                    206
                } else {
                    200
                },
            ),
        ] {
            let response = t.get("GET", &url, &headers).await;
            assert_eq!(response.status().as_u16(), status);
            bytes(response).await;
        }
        assert_eq!(
            t.get("GET", "/product/latest/windows-x86_64", &[])
                .await
                .status(),
            307
        );
        assert_eq!(
            t.get("GET", "/product/1.0.0/windows-x86_64", &[])
                .await
                .status(),
            307
        );
        assert_eq!(
            t.get("GET", "/product/manifest.json", &[]).await.status(),
            200
        );
        let admin_file = format!(
            "/api/v1/admin/applications/{}/releases/{}/artifacts/{}/download",
            t.app, r.id, a.id
        );
        assert_eq!(
            bytes(
                t.admin("GET", &admin_file, Value::Null, Some(&t.token), false)
                    .await
            )
            .await,
            b"0123456789"
        );
        assert_eq!(
            t.state
                .applications
                .get(&t.app)
                .await
                .unwrap()
                .download_count,
            0
        );
        for headers in [
            vec![],
            vec![("range", "bytes=0-3")],
            vec![("range", "bytes=4-"), ("if-range", "\"obsolete\"")],
            vec![("range", "bytes=0-2,4-5")],
            vec![("range", "bytes=-100")],
        ] {
            let response = t.get("GET", &url, &headers).await;
            assert!(response.status().is_success());
            // A disconnect still represents an accepted start.
            drop(response);
        }
        assert_eq!(
            t.state
                .applications
                .get(&t.app)
                .await
                .unwrap()
                .download_count,
            5
        );
        assert_eq!(
            t.state
                .releases
                .get(&t.app, &r.id)
                .await
                .unwrap()
                .download_count,
            5
        );
        assert_eq!(
            t.state.artifacts.list(&t.app, &r.id).await.unwrap()[0].download_count,
            5
        );
        // Reading statistics and restarting infrastructure never reset the value.
        let pool = initialize(&t.config).await.unwrap();
        let restarted = application_state(pool.clone(), &t.config).await.unwrap();
        assert_eq!(
            restarted
                .applications
                .get(&t.app)
                .await
                .unwrap()
                .download_count,
            5
        );
        pool.close().await;
        // Missing storage is rejected before accounting, including Nginx delegation.
        tokio::fs::remove_file(t.config.storage.directory.join(&a.storage_key))
            .await
            .unwrap();
        assert_eq!(t.get("GET", &url, &[]).await.status(), 503);
        assert_eq!(
            t.state
                .applications
                .get(&t.app)
                .await
                .unwrap()
                .download_count,
            5
        );
    }
}

#[tokio::test]
async fn download_counts_roll_up_by_ownership_and_stay_behind_the_admin_gate() {
    let t = TestApp::new(DeliveryMode::Direct).await;
    let old = t.create("1.0.0").await;
    let installer = t.upload(&old, "windows-x86_64", "setup.exe").await;
    let archive = t.upload_type(&old, "any", "bundle.zip", "archive").await;
    t.publish(&old).await;
    let new = t.create("2.0.0").await;
    let binary = t
        .upload_type(&new, "linux-x86_64", "program", "executable")
        .await;
    t.publish(&new).await;
    let draft = t.create("3.0.0").await;
    for (release, artifact, count) in [
        (&old, installer.id.as_str(), 1),
        (&old, archive["id"].as_str().unwrap(), 2),
        (&new, binary["id"].as_str().unwrap(), 3),
    ] {
        let path = format!(
            "/product/releases/{}/artifacts/{artifact}/download",
            release.version
        );
        for _ in 0..count {
            assert_eq!(bytes(t.get("GET", &path, &[]).await).await, b"0123456789");
        }
    }
    let base = format!("/api/v1/admin/applications/{}", t.app);
    for path in [
        base.clone(),
        format!("{base}/releases"),
        format!("{base}/releases/{}/artifacts", old.id),
    ] {
        assert_eq!(
            t.admin("GET", &path, Value::Null, None, false)
                .await
                .status(),
            401
        );
    }
    t.state
        .users
        .create(
            &t.actor,
            "reader",
            Role::Operator,
            Some("reader-password-123".into()),
        )
        .await
        .unwrap();
    let operator = t
        .state
        .authentication
        .login("reader", "reader-password-123".into(), None)
        .await
        .unwrap();
    for token in [&t.token, &operator.token] {
        let app = json_body(t.admin("GET", &base, Value::Null, Some(token), false).await).await;
        assert_eq!(app["download_count"], 6);
        let catalog = json_body(
            t.admin(
                "GET",
                "/api/v1/admin/applications",
                Value::Null,
                Some(token),
                false,
            )
            .await,
        )
        .await;
        assert_eq!(catalog["items"][0]["download_count"], 6);
        let releases = json_body(
            t.admin(
                "GET",
                &format!("{base}/releases?per_page=1&page=2"),
                Value::Null,
                Some(token),
                false,
            )
            .await,
        )
        .await;
        assert_eq!(releases["items"][0]["id"], new.id);
        assert_eq!(releases["items"][0]["download_count"], 3);
        let release = json_body(
            t.admin(
                "GET",
                &format!("{base}/releases/{}", old.id),
                Value::Null,
                Some(token),
                false,
            )
            .await,
        )
        .await;
        assert_eq!(release["download_count"], 3);
        let artifacts = json_body(
            t.admin(
                "GET",
                &format!("{base}/releases/{}/artifacts", old.id),
                Value::Null,
                Some(token),
                false,
            )
            .await,
        )
        .await;
        let artifacts = artifacts.as_array().unwrap();
        assert_eq!(
            artifacts.iter().find(|a| a["id"] == installer.id).unwrap()["download_count"],
            1
        );
        assert_eq!(
            artifacts.iter().find(|a| a["id"] == archive["id"]).unwrap()["download_count"],
            2
        );
    }
    assert_eq!(
        t.state
            .releases
            .get(&t.app, &draft.id)
            .await
            .unwrap()
            .download_count,
        0
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
    assert_eq!(other.download_count, 0);
    assert_eq!(
        t.state
            .applications
            .get(&other.id)
            .await
            .unwrap()
            .download_count,
        0
    );
    let public = json_body(
        t.get("GET", "/api/v1/public/applications/product", &[])
            .await,
    )
    .await;
    assert!(public.get("download_count").is_none());
    assert!(
        public["latest_by_target"]["windows-x86_64"]["artifacts"][0]
            .get("download_count")
            .is_none()
    );
    let stale = t
        .state
        .distribution
        .version("product", "1.0.0", &Default::default(), Some(&installer.id))
        .await
        .unwrap();
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
    assert!(matches!(
        t.state.distribution.record_download(&stale).await,
        Err(veruna_releases::services::error::ServiceError::NotFound)
    ));
    assert_eq!(
        t.get("GET", &t.url(&old, &installer), &[]).await.status(),
        404
    );
    t.state
        .applications
        .set_status(&t.actor, &t.app, Status::Archived)
        .await
        .unwrap();
    assert_eq!(
        t.state
            .applications
            .get(&t.app)
            .await
            .unwrap()
            .download_count,
        6
    );
}

#[tokio::test]
async fn download_counts_increment_atomically_without_changing_publication_or_manifests() {
    let t = TestApp::new(DeliveryMode::Direct).await;
    let r = t.create("1.0.0").await;
    let a = t.upload(&r, "windows-x86_64", "setup.exe").await;
    t.publish(&r).await;
    let before = t.state.releases.get(&t.app, &r.id).await.unwrap();
    let manifest = t.get("GET", "/product/manifest.json", &[]).await;
    let etag = manifest.headers()["etag"].to_str().unwrap().to_owned();
    drop(manifest);
    let download = t
        .state
        .distribution
        .version("product", "1.0.0", &Default::default(), Some(&a.id))
        .await
        .unwrap();
    let results = futures_util::future::join_all(
        (0..32).map(|_| t.state.distribution.record_download(&download)),
    )
    .await;
    for result in results {
        result.unwrap();
    }
    let after = t.state.releases.get(&t.app, &r.id).await.unwrap();
    assert_eq!(after.download_count, 32);
    assert_eq!(
        t.state
            .applications
            .get(&t.app)
            .await
            .unwrap()
            .download_count,
        32
    );
    assert_eq!(
        t.state.artifacts.list(&t.app, &r.id).await.unwrap()[0].download_count,
        32
    );
    assert_eq!(
        (after.revision, after.updated_at, after.published_at),
        (before.revision, before.updated_at, before.published_at)
    );
    assert_eq!(
        t.get("GET", "/product/manifest.json", &[("if-none-match", &etag)])
            .await
            .status(),
        304
    );
    assert!(
        sqlx::query("UPDATE artifact_download_counts SET download_count=-1")
            .execute(&t.state.database)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE artifact_download_counts SET download_count=1.5")
            .execute(&t.state.database)
            .await
            .is_err()
    );
}
