use axum::{
    Router,
    body::{Body, Bytes},
    http::Request,
    response::Response,
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tempfile::{TempDir, tempdir};
use tower::ServiceExt;
use veruna_releases::{
    application_state,
    config::AppConfig,
    domain::{
        application::{NewApplication, Status, Visibility},
        artifact::UploadMetadata,
        release::{self, ReleaseInput},
        user::Role,
    },
    http::{self, AppState},
    infrastructure::{
        secrets,
        storage::{ArtifactStorage, LocalArtifactStorage},
    },
    initialize,
    services::{error::ServiceError, sessions::Identity},
};
struct TestApp {
    dir: TempDir,
    config: AppConfig,
    state: AppState,
    router: Router,
    actor: Identity,
    token: String,
    app: String,
    release: String,
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
        config.storage.max_file_bytes = 1024;
        config.storage.max_request_bytes = 8192;
        config.storage.upload_idle_seconds = 1;
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
                    name: "Product".into(),
                    description: String::new(),
                    visibility: Visibility::Private,
                },
            )
            .await
            .unwrap()
            .id;
        let release = state
            .releases
            .create(&actor, &app, input("1.2.0"))
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
            release,
        }
    }
    fn base(&self) -> String {
        format!("/api/v1/admin/applications/{}/releases", self.app)
    }
    fn detail(&self) -> String {
        format!("{}/{}", self.base(), self.release)
    }
    fn artifacts(&self) -> String {
        format!("{}/artifacts", self.detail())
    }
    async fn request(
        &self,
        method: &str,
        path: &str,
        body: Body,
        content_type: &str,
        token: Option<&str>,
        csrf: bool,
    ) -> Response {
        let mut builder = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", content_type)
            .header("origin", &self.config.auth.public_origin)
            .header("x-veruna-request", "1");
        if let Some(token) = token {
            builder = builder.header("cookie", format!("veruna_session={token}"));
            if csrf {
                builder = builder.header("x-csrf-token", secrets::csrf(token));
            }
        }
        self.router
            .clone()
            .oneshot(builder.body(body).unwrap())
            .await
            .unwrap()
    }
    async fn call(&self, method: &str, path: &str, data: Value, status: u16) -> Value {
        let response = self
            .request(
                method,
                path,
                Body::from(data.to_string()),
                "application/json",
                Some(&self.token),
                true,
            )
            .await;
        if response.status() != status {
            let actual = response.status();
            panic!(
                "{method} {path}: {actual} instead of {status}: {}",
                String::from_utf8_lossy(&raw(response).await)
            );
        }
        if status == 204 {
            return Value::Null;
        }
        json_body(response).await
    }
    async fn upload(&self, filename: &str, target: &str, bytes: &[u8], status: u16) -> Value {
        let response = self
            .request(
                "POST",
                &self.artifacts(),
                Body::from(multipart(filename, target, bytes)),
                "multipart/form-data; boundary=BOUNDARY",
                Some(&self.token),
                true,
            )
            .await;
        let actual = response.status();
        let value = json_body(response).await;
        assert_eq!(actual, status, "{value}");
        value
    }
    async fn operator(&self) -> String {
        self.state
            .users
            .create(
                &self.actor,
                "operator",
                Role::Operator,
                Some("test-password-123".into()),
            )
            .await
            .unwrap();
        self.state
            .authentication
            .login("operator", "test-password-123".into(), None)
            .await
            .unwrap()
            .token
    }
}
fn input(version: &str) -> ReleaseInput {
    ReleaseInput {
        version: version.into(),
        title: None,
        release_notes: String::new(),
    }
}
fn metadata(target: &str) -> UploadMetadata {
    UploadMetadata {
        filename: "installer.bin".into(),
        target: target.into(),
        file_type: None,
        distribution_role: None,
    }
}
fn multipart(filename: &str, target: &str, bytes: &[u8]) -> Vec<u8> {
    let mut data=format!("--BOUNDARY\r\nContent-Disposition: form-data; name=\"target\"\r\n\r\n{target}\r\n--BOUNDARY\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\nContent-Type: application/octet-stream\r\n\r\n").into_bytes();
    data.extend_from_slice(bytes);
    data.extend_from_slice(b"\r\n--BOUNDARY--\r\n");
    data
}
async fn raw(response: Response) -> Vec<u8> {
    response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec()
}
async fn json_body(response: Response) -> Value {
    serde_json::from_slice(&raw(response).await).unwrap()
}
async fn wait_empty(path: &std::path::Path) {
    for _ in 0..100 {
        if std::fs::read_dir(path).unwrap().next().is_none() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("Temporary objects were not reclaimed");
}
#[test]
fn semver_and_format_rules_cover_normalization_precedence_and_open_formats() {
    for value in ["1.0.0", "2.0.0-beta.1", "3.1.0-rc.2", "1.4.0+build.15"] {
        assert!(release::version(value).is_some());
    }
    assert_eq!(release::version(" v1.2.0 ").unwrap().to_string(), "1.2.0");
    for value in [
        "1.2",
        "01.2.3",
        "1.0.0-01",
        "1.0.0+",
        "vv1.0.0",
        "1.0.0-β",
        "1.0.0 ".repeat(50).as_str(),
    ] {
        assert!(release::version(value).is_none(), "{value}");
    }
    assert_eq!(
        release::version_key(&release::version("1.2.0+build.15").unwrap()),
        "1.2.0"
    );
    use veruna_releases::domain::artifact::{file_type, valid_filename};
    assert_eq!(
        file_type("installer.TAR.GZ", None).as_deref(),
        Some("tar.gz")
    );
    assert_eq!(
        file_type("installer.custom", None).as_deref(),
        Some("custom")
    );
    assert_eq!(file_type("installer", Some("BIN")).as_deref(), Some("bin"));
    assert!(file_type("installer.exe", Some("zip")).is_none());
    assert_eq!(file_type("installer", None).as_deref(), Some("bin"));
    for name in [
        "../secret.bin",
        "/etc/passwd",
        "a\\b.exe",
        "..",
        "a\r\nb.exe",
        "",
    ] {
        assert!(!valid_filename(name));
    }
}
#[tokio::test]
async fn release_lifecycle_uniqueness_sort_search_and_archival() {
    let app = TestApp::new().await;
    let created = app
        .call(
            "POST",
            &app.base(),
            json!({"version":" v2.0.0-beta.1 ","title":" Preview ","release_notes":"Changes"}),
            201,
        )
        .await;
    assert_eq!(created["version"], "2.0.0-beta.1");
    assert_eq!(created["status"], "draft");
    assert_eq!(created["title"], "Preview");
    assert!(created["published_at"].is_null());
    assert_eq!(created["artifact_count"], 0);
    let path = format!("{}/{}", app.base(), created["id"].as_str().unwrap());
    assert_eq!(app.call("GET", &path, Value::Null, 200).await, created);
    for version in ["v1.2.0", "1.2.0+other"] {
        assert_eq!(
            app.call("POST", &app.base(), json!({"version":version}), 409)
                .await["error"]["code"],
            "release_version_exists"
        );
    }
    let other = app
        .state
        .applications
        .create(
            &app.actor,
            NewApplication {
                slug: "other".into(),
                name: "Other".into(),
                description: String::new(),
                visibility: Visibility::Private,
            },
        )
        .await
        .unwrap();
    app.state
        .releases
        .create(&app.actor, &other.id, input("1.2.0"))
        .await
        .unwrap();
    let changed = app
        .call(
            "PATCH",
            &path,
            json!({"version":"2.0.0","title":null,"release_notes":"Updated"}),
            200,
        )
        .await;
    assert!(changed["title"].is_null());
    assert_eq!(changed["release_notes"], "Updated");
    assert_eq!(changed["created_at"], created["created_at"]);
    for version in ["1.10.0", "1.9.0", "1.10.0-beta.11", "1.10.0-beta.2"] {
        app.call("POST", &app.base(), json!({"version":version}), 201)
            .await;
    }
    let page = app
        .call(
            "GET",
            &format!("{}?per_page=3&sort=version&order=desc", app.base()),
            Value::Null,
            200,
        )
        .await;
    assert_eq!(page["total"], 6);
    assert_eq!(page["items"][0]["version"], "2.0.0");
    assert_eq!(page["items"][1]["version"], "1.10.0");
    assert_eq!(page["items"][2]["version"], "1.10.0-beta.11");
    assert_eq!(
        app.call(
            "GET",
            &format!("{}?search=beta", app.base()),
            Value::Null,
            200
        )
        .await["total"],
        2
    );
    for invalid in [
        json!({"version":"1.2"}),
        json!({"version":"1.0.0","status":"published"}),
        json!({"version":"3.0.0","title":"a".repeat(121)}),
        json!({"version":"3.0.0","release_notes":"a".repeat(20001)}),
    ] {
        app.call("POST", &app.base(), invalid, 422).await;
    }
    for invalid in [
        json!({}),
        json!({"version":null}),
        json!({"application_id":other.id}),
        json!({"published_at":1}),
    ] {
        app.call("PATCH", &path, invalid, 422).await;
    }
    for query in [
        "page=0",
        "per_page=101",
        "sort=title",
        "order=other",
        "unknown=yes",
    ] {
        app.call("GET", &format!("{}?{query}", app.base()), Value::Null, 422)
            .await;
    }
    app.state
        .applications
        .set_status(&app.actor, &app.app, Status::Archived)
        .await
        .unwrap();
    app.call("POST", &app.base(), json!({"version":"4.0.0"}), 409)
        .await;
    app.call("DELETE", &path, Value::Null, 204).await;
    app.call("GET", &path, Value::Null, 404).await;
}
#[tokio::test]
async fn advertised_platforms_and_formats_are_accepted_for_upload() {
    let app = TestApp::new().await;
    let options = app
        .call("GET", "/api/v1/admin/artifact-options", Value::Null, 200)
        .await;
    assert!(
        options["artifact_types"]
            .as_array()
            .unwrap()
            .contains(&json!("script"))
    );
    for (target, filename, file_type) in [
        ("linux", "install.sh", "sh"),
        ("darwin", "install.sh", "sh"),
        ("windows", "install.ps1", "ps1"),
        ("darwin-universal", "installer.dmg", "dmg"),
        ("linux-x86_64", "program.x86_64", "x86_64"),
        ("linux-x86_64", "program-1.2.0-linux-x86_64", "bin"),
    ] {
        assert!(
            options["targets"]
                .as_array()
                .unwrap()
                .contains(&json!(target))
        );
        assert!(
            options["file_types"]
                .as_array()
                .unwrap()
                .contains(&json!(file_type))
        );
        let artifact = app.upload(filename, target, b"package", 201).await;
        assert_eq!(artifact["target"], target);
        assert_eq!(artifact["filename"], filename);
        assert_eq!(artifact["file_type"], file_type);
    }
    for target in ["windows-universal", "linux-universal"] {
        app.upload("installer.bin", target, b"invalid", 422).await;
    }
}
#[tokio::test]
async fn artifact_stream_hash_download_verify_delete_and_release_cascade() {
    let app = TestApp::new().await;
    let first = app
        .upload("установщик.BIN", "windows-x86_64", b"abc", 201)
        .await;
    assert_eq!(first["size_bytes"], 3);
    assert_eq!(
        first["sha256"],
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(first["file_type"], "bin");
    assert!(first.get("storage_key").is_none());
    let second = app
        .upload("linux.tar.gz", "linux-x86_64", b"linux", 201)
        .await;
    assert_ne!(first["id"], second["id"]);
    assert_eq!(
        app.upload("duplicate.bin", "windows-x86_64", b"other", 409)
            .await["error"]["code"],
        "artifact_exists"
    );
    let release = app.call("GET", &app.detail(), Value::Null, 200).await;
    assert_eq!(release["artifact_count"], 2);
    assert_eq!(release["total_size_bytes"], 8);
    let path = format!("{}/{}", app.artifacts(), first["id"].as_str().unwrap());
    let operator = app.operator().await;
    let response = app
        .request(
            "GET",
            &format!("{path}/download"),
            Body::empty(),
            "",
            Some(&operator),
            false,
        )
        .await;
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.headers()["content-type"],
        "application/octet-stream"
    );
    assert_eq!(response.headers()["content-length"], "3");
    assert!(
        response.headers()["content-disposition"]
            .to_str()
            .unwrap()
            .contains("filename*=UTF-8''%D1")
    );
    assert_eq!(raw(response).await, b"abc");
    assert_eq!(
        app.call("POST", &format!("{path}/verify"), Value::Null, 200)
            .await["valid"],
        true
    );
    let key: String = sqlx::query_scalar("SELECT storage_key FROM artifacts WHERE id=?")
        .bind(first["id"].as_str().unwrap())
        .fetch_one(&app.state.database)
        .await
        .unwrap();
    std::fs::write(app.config.storage.directory.join(&key), b"bad").unwrap();
    assert_eq!(
        app.call("POST", &format!("{path}/verify"), Value::Null, 200)
            .await["valid"],
        false
    );
    app.call("DELETE", &path, Value::Null, 204).await;
    assert!(!app.config.storage.directory.join(key).exists());
    app.call("GET", &format!("{path}/download"), Value::Null, 404)
        .await;
    app.call("DELETE", &path, Value::Null, 404).await;
    app.call("DELETE", &app.detail(), Value::Null, 204).await;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM artifacts")
            .fetch_one(&app.state.database)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        std::fs::read_dir(&app.config.storage.directory)
            .unwrap()
            .count(),
        1
    ); // .tmp directory
}
#[tokio::test]
async fn administrative_gates_csrf_ownership_and_no_public_access() {
    let app = TestApp::new().await;
    let operator = app.operator().await;
    let artifact = app
        .upload("installer.exe", "windows-x86_64", b"abc", 201)
        .await;
    let artifact = format!("{}/{}", app.artifacts(), artifact["id"].as_str().unwrap());
    for (method, path) in [
        ("GET", app.base()),
        ("POST", app.base()),
        ("GET", app.detail()),
        ("PATCH", app.detail()),
        ("DELETE", app.detail()),
        ("POST", app.artifacts()),
        ("GET", app.artifacts()),
        ("GET", format!("{artifact}/download")),
        ("DELETE", artifact.clone()),
        ("POST", format!("{artifact}/verify")),
        ("GET", "/api/v1/admin/artifact-options".into()),
    ] {
        assert_eq!(
            app.request(
                method,
                &path,
                Body::from("{}"),
                "application/json",
                None,
                true
            )
            .await
            .status(),
            401,
            "{method} {path}"
        );
        assert_eq!(
            app.request(
                method,
                &path,
                Body::from("{}"),
                "application/json",
                Some(&operator),
                true
            )
            .await
            .status(),
            if method == "GET" { 200 } else { 403 },
            "{method} {path}"
        );
        if method != "GET" {
            assert_eq!(
                app.request(
                    method,
                    &path,
                    Body::from("{}"),
                    "application/json",
                    Some(&app.token),
                    false
                )
                .await
                .status(),
                403
            );
        }
    }
    let other = uuid::Uuid::new_v4().to_string();
    for path in [
        app.detail().replace(&app.app, &other),
        app.artifacts().replace(&app.release, &other),
        format!("{artifact}/download").replace(&app.app, &other),
        format!("{artifact}/download").replace(&app.release, &other),
    ] {
        app.call("GET", &path, Value::Null, 404).await;
    }
    app.call(
        "DELETE",
        &artifact.replace(&app.app, &other),
        Value::Null,
        404,
    )
    .await;
    app.call("GET", "/api/v1/artifacts", Value::Null, 404).await;
    app.call(
        "GET",
        "/api/v1/admin/applications/bad/releases",
        Value::Null,
        404,
    )
    .await;
    app.call("GET", "/data/artifacts/secret.blob", Value::Null, 404)
        .await;
}
#[tokio::test]
async fn invalid_oversized_truncated_and_cancelled_uploads_are_not_visible() {
    let app = TestApp::new().await;
    for filename in ["../escape.bin", "a\\b.bin", "invalid.", ""] {
        app.upload(filename, "any", b"abc", 422).await;
    }
    app.upload("file.bin", "tauri-windows-x64", b"abc", 422)
        .await;
    app.upload("empty.bin", "any", b"", 422).await;
    app.upload("large.bin", "any", &vec![1; 1025], 413).await;
    app.upload("request.bin", "any", &vec![1; 9000], 413).await;
    let mut truncated = multipart("broken.bin", "any", b"partial");
    truncated.truncate(truncated.len() - 15);
    let response = app
        .request(
            "POST",
            &app.artifacts(),
            Body::from(truncated),
            "multipart/form-data; boundary=BOUNDARY",
            Some(&app.token),
            true,
        )
        .await;
    assert_eq!(response.status(), 422);
    let head = multipart("disconnect.bin", "any", b"partial");
    let stream = futures_util::stream::iter([
        Ok::<_, std::io::Error>(Bytes::from(head[..head.len() - 16].to_vec())),
        Err(std::io::Error::other("connection reset")),
    ]);
    assert_eq!(
        app.request(
            "POST",
            &app.artifacts(),
            Body::from_stream(stream),
            "multipart/form-data; boundary=BOUNDARY",
            Some(&app.token),
            true
        )
        .await
        .status(),
        422
    );
    // Cancellation after a temporary file has actually been written.
    let service = app.state.artifacts.clone();
    let actor = app.actor.clone();
    let application = app.app.clone();
    let release = app.release.clone();
    let (ready, received) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let mut upload = service
            .begin(&actor, &application, &release, metadata("any"))
            .await
            .unwrap();
        upload.write(b"partial").await.unwrap();
        ready.send(()).unwrap();
        std::future::pending::<()>().await;
        drop(upload);
    });
    received.await.unwrap();
    task.abort();
    let _ = task.await;
    wait_empty(&app.config.storage.temporary_directory()).await;
    assert_eq!(
        app.call("GET", &app.artifacts(), Value::Null, 200).await,
        json!([])
    );
    assert_eq!(
        std::fs::read_dir(&app.config.storage.directory)
            .unwrap()
            .count(),
        1
    );
    assert!(!app.dir.path().join("escape.bin").exists());
}
#[tokio::test]
async fn multipart_idle_timeout_metadata_limits_and_trailing_fields() {
    let app = TestApp::new().await;
    let head = multipart("slow.bin", "any", b"partial");
    let partial = Bytes::from(head[..head.len() - 16].to_vec());
    let stream = futures_util::stream::once(async move { Ok::<_, std::io::Error>(partial) })
        .chain(futures_util::stream::pending());
    assert_eq!(
        app.request(
            "POST",
            &app.artifacts(),
            Body::from_stream(stream),
            "multipart/form-data; boundary=BOUNDARY",
            Some(&app.token),
            true
        )
        .await
        .status(),
        408
    );
    let mut trailing = multipart("file.bin", "any", b"content");
    trailing.truncate(trailing.len() - 14);
    trailing.extend_from_slice(b"--BOUNDARY\r\nContent-Disposition: form-data; name=\"extra\"\r\n\r\nx\r\n--BOUNDARY--\r\n");
    assert_eq!(
        app.request(
            "POST",
            &app.artifacts(),
            Body::from(trailing),
            "multipart/form-data; boundary=BOUNDARY",
            Some(&app.token),
            true
        )
        .await
        .status(),
        422
    );
    app.upload("file.bin", &"x".repeat(129), b"abc", 422).await;
    // Malformed preamble/header cannot grow the multipart parser to file-size limits.
    let mut big = app.config.storage.clone();
    big.max_file_bytes = 100000;
    big.max_request_bytes = 120000;
    let mut state = app.state.clone();
    state.storage_config = big;
    let response = http::router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(app.artifacts())
                .header("content-type", "multipart/form-data; boundary=BOUNDARY")
                .header("origin", &app.config.auth.public_origin)
                .header("x-veruna-request", "1")
                .header("x-csrf-token", secrets::csrf(&app.token))
                .header("cookie", format!("veruna_session={}", app.token))
                .body(Body::from(vec![b'x'; 100000]))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 422);
    wait_empty(&app.config.storage.temporary_directory()).await;
    assert_eq!(
        app.call("GET", &app.artifacts(), Value::Null, 200).await,
        json!([])
    );
}
use futures_util::StreamExt;
#[tokio::test]
async fn concurrent_uploads_live_authorization_and_deleted_release_are_rechecked() {
    let app = TestApp::new().await;
    let (one, two) = tokio::join!(
        app.state
            .releases
            .create(&app.actor, &app.app, input("5.0.0")),
        app.state
            .releases
            .create(&app.actor, &app.app, input("v5.0.0+build"))
    );
    assert!(matches!(
        (&one, &two),
        (Ok(_), Err(ServiceError::ReleaseVersionConflict))
            | (Err(ServiceError::ReleaseVersionConflict), Ok(_))
    ));
    let mut one = app
        .state
        .artifacts
        .begin(&app.actor, &app.app, &app.release, metadata("any"))
        .await
        .unwrap();
    let mut two = app
        .state
        .artifacts
        .begin(&app.actor, &app.app, &app.release, metadata("any"))
        .await
        .unwrap();
    one.write(b"one").await.unwrap();
    two.write(b"two").await.unwrap();
    let (one, two) = tokio::join!(
        app.state.artifacts.complete(&app.actor, one),
        app.state.artifacts.complete(&app.actor, two)
    );
    assert!(matches!(
        (&one, &two),
        (Ok(_), Err(ServiceError::ArtifactConflict)) | (Err(ServiceError::ArtifactConflict), Ok(_))
    ));
    let mut deleted = app
        .state
        .artifacts
        .begin(&app.actor, &app.app, &app.release, metadata("linux-x86_64"))
        .await
        .unwrap();
    deleted.write(b"bytes").await.unwrap();
    app.state
        .releases
        .delete(&app.actor, &app.app, &app.release)
        .await
        .unwrap();
    assert!(matches!(
        app.state.artifacts.complete(&app.actor, deleted).await,
        Err(ServiceError::NotFound)
    ));
    let revoked = app
        .state
        .artifacts
        .begin(
            &app.actor,
            &app.app,
            &app.release,
            metadata("darwin-aarch64"),
        )
        .await;
    assert!(revoked.is_err()); // Deleted parent can never accept another upload.
    let new = app
        .state
        .releases
        .create(&app.actor, &app.app, input("6.0.0"))
        .await
        .unwrap();
    let mut pending = app
        .state
        .artifacts
        .begin(&app.actor, &app.app, &new.id, metadata("any"))
        .await
        .unwrap();
    pending.write(b"bytes").await.unwrap();
    app.state.sessions.logout(&app.actor).await.unwrap();
    assert!(matches!(
        app.state.artifacts.complete(&app.actor, pending).await,
        Err(ServiceError::Unauthorized)
    ));
    assert!(matches!(
        app.state
            .releases
            .create(&app.actor, &app.app, input("7.0.0"))
            .await,
        Err(ServiceError::Unauthorized)
    ));
    wait_empty(&app.config.storage.temporary_directory()).await;
}
#[tokio::test]
async fn sqlite_failure_and_crash_recovery_preserve_references_and_reclaim_orphans() {
    let app = TestApp::new().await;
    let valid = app.upload("file.bin", "any", b"abc", 201).await;
    let mut pending = app
        .state
        .artifacts
        .begin(&app.actor, &app.app, &app.release, metadata("linux-x86_64"))
        .await
        .unwrap();
    pending.write(b"abc").await.unwrap();
    // Simulate a DB failure after file promotion, before metadata can commit.
    sqlx::query("CREATE TRIGGER fail_artifact BEFORE INSERT ON artifacts BEGIN SELECT RAISE(ABORT,'test failure'); END").execute(&app.state.database).await.unwrap();
    assert!(
        app.state
            .artifacts
            .complete(&app.actor, pending)
            .await
            .is_err()
    );
    sqlx::query("DROP TRIGGER fail_artifact")
        .execute(&app.state.database)
        .await
        .unwrap();
    wait_empty(&app.config.storage.temporary_directory()).await;
    let orphan = app
        .config
        .storage
        .directory
        .join(format!("{}.blob", uuid::Uuid::new_v4().simple()));
    std::fs::write(&orphan, b"crash after install").unwrap();
    let temporary = app
        .config
        .storage
        .temporary_directory()
        .join(format!("{}.tmp", uuid::Uuid::new_v4().simple()));
    std::fs::write(&temporary, b"crash during write").unwrap();
    let arbitrary = app.config.storage.directory.join("operator-notes.txt");
    std::fs::write(&arbitrary, b"keep").unwrap();
    app.state.database.close().await;
    let pool = initialize(&app.config).await.unwrap();
    assert!(!orphan.exists());
    assert!(!temporary.exists());
    assert!(arbitrary.exists());
    let state = application_state(pool.clone(), &app.config).await.unwrap();
    assert_eq!(
        state
            .artifacts
            .download(&app.app, &app.release, valid["id"].as_str().unwrap())
            .await
            .unwrap()
            .0
            .size_bytes,
        3
    );
    let key: String = sqlx::query_scalar("SELECT storage_key FROM artifacts WHERE id=?")
        .bind(valid["id"].as_str().unwrap())
        .fetch_one(&pool)
        .await
        .unwrap();
    pool.close().await;
    std::fs::remove_file(app.config.storage.directory.join(key)).unwrap();
    assert!(initialize(&app.config).await.is_err()); // Never silently discard a missing referenced file.
}
#[cfg(unix)]
#[tokio::test]
async fn storage_rejects_traversal_symlinks_and_overwrite() {
    let app = TestApp::new().await;
    let storage = LocalArtifactStorage::new(&app.config.storage);
    let outside = app.dir.path().join("secret.bin");
    std::fs::write(&outside, b"private").unwrap();
    for key in [
        "../secret.bin",
        "/etc/passwd",
        "..%2fsecret.bin",
        "bad.blob",
    ] {
        assert!(storage.open(key, 7).await.is_err());
        assert!(storage.remove(key).await.is_err());
    }
    let key = format!("{}.blob", uuid::Uuid::new_v4().simple());
    std::os::unix::fs::symlink(&outside, app.config.storage.directory.join(&key)).unwrap();
    assert!(storage.open(&key, 7).await.is_err());
    storage.remove(&key).await.unwrap();
    assert_eq!(std::fs::read(outside).unwrap(), b"private");
    let mut pending = storage.begin().await.unwrap();
    pending.write(b"new").await.unwrap();
    let destination = app.config.storage.directory.join(pending.storage_key());
    std::fs::write(&destination, b"existing").unwrap();
    assert!(storage.promote(&mut pending).await.is_err());
    drop(pending);
    wait_empty(&app.config.storage.temporary_directory()).await;
    assert_eq!(std::fs::read(destination).unwrap(), b"existing");
}

#[tokio::test]
async fn request_stream_is_written_before_end_and_upload_limit_overrides_small_json_limit() {
    let app = TestApp::new().await;
    let mut config = app.config.storage.clone();
    config.max_file_bytes = 512 * 1024;
    config.max_request_bytes = 600 * 1024;
    let storage = LocalArtifactStorage::new(&config);
    let mut state = app.state.clone();
    state.storage_config = config;
    state.artifacts = veruna_releases::services::artifact::ArtifactManagement::new(
        veruna_releases::repositories::artifact::ArtifactsRepository::new(state.database.clone()),
        veruna_releases::repositories::release::ReleasesRepository::new(state.database.clone()),
        storage,
    );
    let router = http::router(state);
    let (sender, receiver) = tokio::sync::mpsc::channel::<Result<Bytes, std::io::Error>>(1);
    let stream = futures_util::stream::unfold(receiver, |mut receiver| async {
        receiver.recv().await.map(|item| (item, receiver))
    });
    let request = Request::builder()
        .method("POST")
        .uri(app.artifacts())
        .header("content-type", "multipart/form-data; boundary=BOUNDARY")
        .header("origin", &app.config.auth.public_origin)
        .header("x-veruna-request", "1")
        .header("cookie", format!("veruna_session={}", app.token))
        .header("x-csrf-token", secrets::csrf(&app.token))
        .body(Body::from_stream(stream))
        .unwrap();
    let task = tokio::spawn(async move { router.oneshot(request).await.unwrap() });
    let full = multipart("stream.bin", "any", &vec![b'x'; 64 * 1024]);
    sender
        .send(Ok(Bytes::copy_from_slice(&full[..full.len() - 16])))
        .await
        .unwrap();
    let temporary = app.config.storage.temporary_directory();
    let mut has_bytes = false;
    for _ in 0..100 {
        has_bytes = std::fs::read_dir(&temporary)
            .unwrap()
            .any(|entry| entry.unwrap().metadata().unwrap().len() > 32 * 1024);
        if has_bytes {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    if !has_bytes && task.is_finished() {
        let response = task.await.unwrap();
        let status = response.status();
        panic!(
            "Upload finished before receiving the full body: {status}, {:?}",
            json_body(response).await
        );
    }
    assert!(
        has_bytes,
        "Server must write chunks before the request finishes; temporary sizes: {:?}",
        std::fs::read_dir(&temporary)
            .unwrap()
            .map(|e| e.unwrap().metadata().unwrap().len())
            .collect::<Vec<_>>()
    );
    assert!(!task.is_finished());
    for _ in 0..3 {
        sender
            .send(Ok(Bytes::from(vec![b'x'; 64 * 1024])))
            .await
            .unwrap();
    }
    sender
        .send(Ok(Bytes::copy_from_slice(&full[full.len() - 16..])))
        .await
        .unwrap();
    drop(sender);
    let response = task.await.unwrap();
    assert_eq!(response.status(), 201);
    let artifact = json_body(response).await;
    assert_eq!(artifact["size_bytes"], 256 * 1024);
    use sha2::{Digest, Sha256};
    assert_eq!(
        artifact["sha256"],
        format!("{:x}", Sha256::digest(vec![b'x'; 256 * 1024]))
    );
}

#[tokio::test]
async fn explicit_extensionless_formats_database_guards_and_uncompressed_streams() {
    let app = TestApp::new().await;
    let data=b"--BOUNDARY\r\nContent-Disposition: form-data; name=\"target\"\r\n\r\nany\r\n--BOUNDARY\r\nContent-Disposition: form-data; name=\"file_type\"\r\n\r\nCUSTOM\r\n--BOUNDARY\r\nContent-Disposition: form-data; name=\"file\"; filename=\"installer\"\r\n\r\nabcdefghijklmnopqrstuvwxyz0123456789\r\n--BOUNDARY--\r\n";
    let response = app
        .request(
            "POST",
            &app.artifacts(),
            Body::from(data.as_slice()),
            "multipart/form-data; boundary=BOUNDARY",
            Some(&app.token),
            true,
        )
        .await;
    assert_eq!(response.status(), 201);
    let artifact = json_body(response).await;
    assert_eq!(artifact["file_type"], "custom");
    let path = format!(
        "{}/{}/download",
        app.artifacts(),
        artifact["id"].as_str().unwrap()
    );
    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri(path)
                .header("cookie", format!("veruna_session={}", app.token))
                .header("accept-encoding", "gzip")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert!(response.headers().get("content-encoding").is_none());
    assert_eq!(raw(response).await, b"abcdefghijklmnopqrstuvwxyz0123456789");
    for sql in [
        "UPDATE releases SET id='00000000-0000-0000-0000-000000000000'",
        "UPDATE releases SET application_id='00000000-0000-0000-0000-000000000000'",
        "UPDATE releases SET created_at=0",
        "UPDATE releases SET status='published'",
        "UPDATE releases SET published_at=1",
        "UPDATE releases SET version_key='9.9.9'",
        "UPDATE artifacts SET filename='changed'",
    ] {
        assert!(
            sqlx::query(sql).execute(&app.state.database).await.is_err(),
            "{sql}"
        );
    }
}

#[cfg(unix)]
#[tokio::test]
async fn deletion_failure_keeps_no_downloadable_record_and_startup_reclaims_file() {
    use std::os::unix::fs::PermissionsExt;
    let app = TestApp::new().await;
    let artifact = app.upload("file.bin", "any", b"abc", 201).await;
    let key: String = sqlx::query_scalar("SELECT storage_key FROM artifacts WHERE id=?")
        .bind(artifact["id"].as_str().unwrap())
        .fetch_one(&app.state.database)
        .await
        .unwrap();
    let permissions = std::fs::metadata(&app.config.storage.directory)
        .unwrap()
        .permissions();
    struct Restore(std::path::PathBuf, std::fs::Permissions);
    impl Drop for Restore {
        fn drop(&mut self) {
            let _ = std::fs::set_permissions(&self.0, self.1.clone());
        }
    }
    let restore = Restore(app.config.storage.directory.clone(), permissions);
    std::fs::set_permissions(
        &app.config.storage.directory,
        std::fs::Permissions::from_mode(0o500),
    )
    .unwrap();
    let path = format!("{}/{}", app.artifacts(), artifact["id"].as_str().unwrap());
    app.call("DELETE", &path, Value::Null, 204).await;
    app.call("GET", &format!("{path}/download"), Value::Null, 404)
        .await;
    drop(restore);
    app.state.database.close().await;
    let pool = initialize(&app.config).await.unwrap();
    assert!(!app.config.storage.directory.join(key).exists());
    pool.close().await;
}
