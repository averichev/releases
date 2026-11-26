use axum::{
    Router,
    body::{Body, Bytes},
    extract::ConnectInfo,
    http::Request,
    response::Response,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    io,
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tempfile::{TempDir, tempdir};
use tower::ServiceExt;
use veruna_releases::{
    application_state,
    config::AppConfig,
    domain::{
        application::{ApplicationChanges, NewApplication, Status, Visibility},
        artifact::UploadMetadata,
        release::ReleaseInput,
        user::Role,
    },
    http::{self, AppState},
    infrastructure::secrets,
    initialize,
    services::sessions::Identity,
};

const CONTENT: &[u8] = b"ready executable bytes";
const NOTES: &str = "## Исправления\n- Исправлен повтор загрузки.";
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
#[derive(Clone)]
struct Part {
    name: String,
    filename: String,
    bytes: Vec<u8>,
    content_type: &'static str,
}
impl Part {
    fn file(name: &str, filename: &str, bytes: &[u8]) -> Self {
        Self {
            name: name.into(),
            filename: filename.into(),
            bytes: bytes.into(),
            content_type: "application/octet-stream",
        }
    }
}
fn metadata(version: &str) -> Value {
    json!({"version":version,"notes":NOTES,"artifacts":[
        {"target":"linux-x86_64","distribution_role":"executable","file_type":"bin","file":"linux"},
        {"target":"windows-x86_64","distribution_role":"installer","file_type":"exe","file":"windows"}
    ]})
}
fn files() -> Vec<Part> {
    vec![
        Part::file("linux", "product-linux-x86_64", CONTENT),
        Part::file("windows", "product.exe", b"windows package"),
    ]
}
fn wire(input: &Value, parts: &[Part]) -> Vec<u8> {
    let mut all = vec![Part {
        name: "release".into(),
        filename: "publish.json".into(),
        bytes: input.to_string().into(),
        content_type: "application/json",
    }];
    all.extend_from_slice(parts);
    raw_wire(&all)
}
fn raw_wire(parts: &[Part]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for part in parts {
        bytes.extend_from_slice(format!("--BATCH\r\nContent-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\nContent-Type: {}\r\n\r\n",part.name,part.filename,part.content_type).as_bytes());
        bytes.extend_from_slice(&part.bytes);
        bytes.extend_from_slice(b"\r\n");
    }
    bytes.extend_from_slice(b"--BATCH--\r\n");
    bytes
}
async fn value(response: Response, status: u16) -> Value {
    let actual = response.status();
    assert_eq!(response.headers().get("cache-control").unwrap(), "no-store");
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(actual, status, "{}", String::from_utf8_lossy(&bytes));
    if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    }
}
struct TestApp {
    _dir: TempDir,
    config: AppConfig,
    state: AppState,
    router: Router,
    actor: Identity,
    session: String,
    app: String,
    token: String,
}
impl TestApp {
    async fn new() -> Self {
        let dir = tempdir().unwrap();
        let mut config = AppConfig::default();
        config.database.path = dir.path().join("db.sqlite");
        config.storage.directory = dir.path().join("objects");
        config.storage.icons_directory = dir.path().join("icons");
        config.storage.max_file_bytes = 32 * 1024;
        config.storage.max_request_bytes = 128 * 1024;
        config.storage.upload_idle_seconds = 1;
        let state = application_state(initialize(&config).await.unwrap(), &config)
            .await
            .unwrap();
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
                    visibility: Visibility::Public,
                },
            )
            .await
            .unwrap()
            .id;
        let token = state
            .api_tokens
            .create(&actor, &app, "CI Product", None)
            .await
            .unwrap()
            .secret;
        let router = http::router(state.clone());
        Self {
            _dir: dir,
            config,
            state,
            router,
            actor,
            session: login.token,
            app,
            token,
        }
    }
    fn base(&self) -> String {
        format!("/api/v1/admin/applications/{}", self.app)
    }
    fn request(&self, slug: &str, token: Option<&str>, body: Body) -> Request<Body> {
        let mut r = Request::builder()
            .method("POST")
            .uri(format!("/api/v1/publish/{slug}"))
            .header("content-type", "multipart/form-data; boundary=BATCH");
        if let Some(token) = token {
            r = r.header("authorization", format!("Bearer {token}"));
        }
        r.body(body).unwrap()
    }
    async fn send(&self, input: &Value, parts: &[Part]) -> Response {
        self.router
            .clone()
            .oneshot(self.request("product", Some(&self.token), Body::from(wire(input, parts))))
            .await
            .unwrap()
    }
    async fn admin(
        &self,
        method: &str,
        path: &str,
        body: Value,
        session: Option<&str>,
        csrf: bool,
    ) -> Response {
        let mut r = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json")
            .header("origin", "http://127.0.0.1:8080")
            .header("x-veruna-request", "1");
        if let Some(session) = session {
            r = r.header("cookie", format!("veruna_session={session}"));
            if csrf {
                r = r.header("x-csrf-token", secrets::csrf(session));
            }
        }
        self.router
            .clone()
            .oneshot(r.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap()
    }
    async fn count(&self, table: &str) -> i64 {
        sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
            .fetch_one(&self.state.database)
            .await
            .unwrap()
    }
    async fn download(&self, path: &str, expected: &[u8]) {
        let r = self
            .router
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        if r.status() == 307 {
            let url = r.headers()["location"].to_str().unwrap().to_owned();
            let r = self
                .router
                .clone()
                .oneshot(Request::builder().uri(url).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(r.status(), 200);
            assert_eq!(
                &r.into_body().collect().await.unwrap().to_bytes()[..],
                expected
            );
        } else {
            assert_eq!(r.status(), 200);
            assert_eq!(
                &r.into_body().collect().await.unwrap().to_bytes()[..],
                expected
            );
        }
    }
    async fn history(&self) -> Value {
        value(
            self.admin(
                "GET",
                &format!("{}/publication-history", self.base()),
                Value::Null,
                Some(&self.session),
                false,
            )
            .await,
            200,
        )
        .await
    }
    async fn restart(&mut self) {
        self.state.database.close().await;
        self.state = application_state(initialize(&self.config).await.unwrap(), &self.config)
            .await
            .unwrap();
        self.router = http::router(self.state.clone());
    }
}

#[tokio::test]
async fn tokens_require_admin_session_csrf_allow_rotation_and_return_secret_only_once() {
    let t = TestApp::new().await;
    let path = format!("{}/publication-tokens", t.base());
    value(t.admin("GET", &path, Value::Null, None, false).await, 401).await;
    value(
        t.admin(
            "POST",
            &path,
            json!({"name":"another"}),
            Some(&t.session),
            false,
        )
        .await,
        403,
    )
    .await;
    let (operator, _) = t
        .state
        .users
        .create(
            &t.actor,
            "operator",
            Role::Operator,
            Some("test-password-123".into()),
        )
        .await
        .unwrap();
    let login = t
        .state
        .authentication
        .login(&operator.username, "test-password-123".into(), None)
        .await
        .unwrap();
    for method in ["GET", "POST"] {
        value(
            t.admin(
                method,
                &path,
                json!({"name":"forbidden"}),
                Some(&login.token),
                true,
            )
            .await,
            403,
        )
        .await;
    }
    let issued = value(
        t.admin(
            "POST",
            &path,
            json!({"name":"Replacement","expires_at":secrets::now()+86400}),
            Some(&t.session),
            true,
        )
        .await,
        201,
    )
    .await;
    let secret = issued["token"].as_str().unwrap();
    assert!(secret.starts_with("vrp_"));
    assert_eq!(secret.len(), 68);
    let stored: String = sqlx::query_scalar("SELECT token_hash FROM api_tokens WHERE id=?")
        .bind(issued["id"].as_str().unwrap())
        .fetch_one(&t.state.database)
        .await
        .unwrap();
    assert_eq!(stored, secrets::digest(secret));
    let list = value(
        t.admin("GET", &path, Value::Null, Some(&t.session), false)
            .await,
        200,
    )
    .await;
    assert_eq!(list.as_array().unwrap().len(), 2);
    assert!(!list.to_string().contains(secret));
    assert!(!list.to_string().contains("token_hash"));
    assert!(
        list.as_array()
            .unwrap()
            .iter()
            .all(|row| row["token"].is_null() && row["state"] == "active")
    );
    let denied = value(
        t.admin(
            "POST",
            &path,
            json!({"name":"third"}),
            Some(&t.session),
            true,
        )
        .await,
        409,
    )
    .await;
    assert_eq!(denied["error"]["code"], "api_token_limit");
    let old_id: String = sqlx::query_scalar("SELECT id FROM api_tokens WHERE token_hash=?")
        .bind(secrets::digest(&t.token))
        .fetch_one(&t.state.database)
        .await
        .unwrap();
    let revoke = format!("{path}/{old_id}/revoke");
    value(
        t.admin("POST", &revoke, json!({}), Some(&login.token), true)
            .await,
        403,
    )
    .await;
    value(
        t.admin("POST", &revoke, json!({}), Some(&t.session), false)
            .await,
        403,
    )
    .await;
    value(
        t.admin("POST", &revoke, json!({}), Some(&t.session), true)
            .await,
        204,
    )
    .await;
    value(
        t.admin("POST", &revoke, json!({}), Some(&t.session), true)
            .await,
        204,
    )
    .await;
    value(t.send(&metadata("1.0.0"), &files()).await, 401).await;
    let r = t
        .router
        .clone()
        .oneshot(t.request(
            "product",
            Some(secret),
            Body::from(wire(&metadata("1.0.0"), &files())),
        ))
        .await
        .unwrap();
    value(r, 201).await;
    value(
        t.admin(
            "POST",
            &path,
            json!({"name":"Next"}),
            Some(&t.session),
            true,
        )
        .await,
        201,
    )
    .await;
    assert_eq!(t.count("api_tokens").await, 3);
}

#[tokio::test]
async fn installation_scripts_resolve_to_the_selected_os_even_with_the_same_filename() {
    let t = TestApp::new().await;
    let legacy = json!({"version":"0.9.0","notes":"","artifacts":[
        {"target":"linux","distribution_role":"installer","file_type":"sh","file":"legacy"}
    ]});
    let legacy_bytes = b"#!/bin/sh\n# Original installer\n";
    let original = value(
        t.send(&legacy, &[Part::file("legacy", "install.sh", legacy_bytes)])
            .await,
        201,
    )
    .await;
    let scripts = [
        (
            "linux",
            "sh",
            "install.sh",
            b"#!/bin/sh\n# Linux\n".as_slice(),
        ),
        ("darwin", "sh", "install.sh", b"#!/bin/sh\n# macOS\n"),
        ("windows", "ps1", "install.ps1", b"# Windows\r\n"),
    ];
    let artifacts: Vec<_> = scripts.iter().map(|(os, format, _, _)| {
        json!({"target":os,"distribution_role":"script","file_type":format,"file":os})
    }).collect();
    let parts: Vec<_> = scripts
        .iter()
        .map(|(os, _, name, bytes)| Part::file(os, name, bytes))
        .collect();
    value(
        t.send(
            &json!({"version":"1.0.0","notes":"","artifacts":artifacts}),
            &parts,
        )
        .await,
        201,
    )
    .await;
    for (os, format, _, bytes) in scripts {
        t.download(
            &format!("/product/latest/{os}?type=script&format={format}"),
            bytes,
        )
        .await;
    }
    t.download("/product/latest/linux?format=sh", legacy_bytes)
        .await;
    t.download(
        "/product/latest/linux?type=installer&format=sh",
        legacy_bytes,
    )
    .await;
    assert_eq!(
        value(
            t.send(&legacy, &[Part::file("legacy", "install.sh", legacy_bytes)])
                .await,
            200
        )
        .await,
        original
    );
    for path in [
        "/product/latest/windows?type=script&format=sh",
        "/product/latest/any?type=script&format=sh",
        "/product/latest/linux-x86_64?type=script&format=sh",
    ] {
        let response = t
            .router
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), 404, "{path}");
    }
}

#[tokio::test]
async fn multi_target_publication_retries_with_other_token_and_transport_then_survives_restart() {
    let mut t = TestApp::new().await;
    let mut input = metadata(" v1.2.0+ci.7 ");
    input["artifacts"][0]["sha256"] = hash(CONTENT).into();
    input["artifacts"][0]["size_bytes"] = CONTENT.len().into();
    let first = value(t.send(&input, &files()).await, 201).await;
    assert_eq!(first["version"], "1.2.0+ci.7");
    assert_eq!(first["status"], "published");
    t.download(
        "/product/latest?target=linux-x86_64&type=executable",
        CONTENT,
    )
    .await;
    t.download(
        "/product/latest?target=windows-x86_64&format=exe",
        b"windows package",
    )
    .await;
    let second = t
        .state
        .api_tokens
        .create(&t.actor, &t.app, "Rotation", None)
        .await
        .unwrap();
    let mut renamed = metadata("1.2.0+ci.7");
    renamed["artifacts"][0]["file"] = "renamed".into();
    renamed["artifacts"].as_array_mut().unwrap().reverse();
    let mut parts = files();
    parts[0].name = "renamed".into();
    parts.reverse();
    let r = t
        .router
        .clone()
        .oneshot(t.request(
            "product",
            Some(&second.secret),
            Body::from(wire(&renamed, &parts)),
        ))
        .await
        .unwrap();
    assert_eq!(value(r, 200).await, first);
    assert_eq!(t.count("releases").await, 1);
    assert_eq!(t.count("publication_history").await, 1);
    assert_eq!(t.count("api_publications").await, 1);
    assert_eq!(t.count("artifacts").await, 2);
    let history = t.history().await;
    assert_eq!(history[0]["author"], "Токен CI Product");
    assert!(history[0]["user_id"].is_null());
    assert!(history[0]["username"].is_null());
    let token_id = history[0]["token_id"].as_str().unwrap();
    t.state
        .api_tokens
        .revoke(&t.actor, &t.app, token_id)
        .await
        .unwrap();
    assert_eq!(t.history().await, history);
    let release = t
        .state
        .releases
        .get(&t.app, first["release_id"].as_str().unwrap())
        .await
        .unwrap();
    assert_eq!(release.release_notes, NOTES);
    let artifacts = t.state.artifacts.list(&t.app, &release.id).await.unwrap();
    assert_eq!(
        artifacts
            .iter()
            .find(|a| a.target == "linux-x86_64")
            .unwrap()
            .sha256,
        hash(CONTENT)
    );
    // Simulate process-crash orphans, then verify the standard recovery path.
    tokio::fs::write(
        t.config
            .storage
            .directory
            .join("00000000000000000000000000000000.blob"),
        b"orphan",
    )
    .await
    .unwrap();
    tokio::fs::write(
        t.config
            .storage
            .temporary_directory()
            .join("00000000000000000000000000000000.tmp"),
        b"partial",
    )
    .await
    .unwrap();
    t.restart().await;
    assert_eq!(t.history().await, history);
    assert_eq!(
        std::fs::read_dir(t.config.storage.directory.clone())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.path().extension().is_some_and(|e| e == "blob"))
            .count(),
        2
    );
    assert_eq!(
        std::fs::read_dir(t.config.storage.temporary_directory())
            .unwrap()
            .count(),
        0
    );
    for a in artifacts {
        assert_eq!(
            hash(
                &tokio::fs::read(t.config.storage.directory.join(a.storage_key))
                    .await
                    .unwrap()
            ),
            a.sha256
        );
    }
    t.download(
        "/product/latest?target=linux-x86_64&type=executable",
        CONTENT,
    )
    .await;
}

#[tokio::test]
async fn original_manifest_is_immutable_after_admin_addition_and_manual_drafts_conflict() {
    let t = TestApp::new().await;
    let input = metadata("1.0.0");
    let first = value(t.send(&input, &files()).await, 201).await;
    let id = first["release_id"].as_str().unwrap();
    let mut upload = t
        .state
        .artifacts
        .begin(
            &t.actor,
            &t.app,
            id,
            UploadMetadata {
                filename: "product-linux-aarch64".into(),
                target: "linux-aarch64".into(),
                file_type: Some("bin".into()),
                distribution_role: Some("executable".into()),
            },
        )
        .await
        .unwrap();
    upload.write(b"arm package").await.unwrap();
    t.state.artifacts.complete(&t.actor, upload).await.unwrap();
    assert_eq!(value(t.send(&input, &files()).await, 200).await, first);
    let mut extended = input.clone();
    extended["artifacts"].as_array_mut().unwrap().push(json!({"file":"arm","target":"linux-aarch64","file_type":"bin","distribution_role":"executable"}));
    let mut extended_files = files();
    extended_files.push(Part::file("arm", "product-linux-aarch64", b"arm package"));
    value(t.send(&extended, &extended_files).await, 409).await;
    for change in ["notes", "version", "bytes"] {
        let mut changed = input.clone();
        let mut parts = files();
        match change {
            "notes" => changed["notes"] = "changed".into(),
            "version" => changed["version"] = "1.0.0+another".into(),
            _ => parts[0].bytes = b"changed executable".to_vec(),
        }
        value(t.send(&changed, &parts).await, 409).await;
    }
    let draft = t
        .state
        .releases
        .create(
            &t.actor,
            &t.app,
            ReleaseInput {
                version: "2.0.0".into(),
                title: None,
                release_notes: NOTES.into(),
            },
        )
        .await
        .unwrap();
    value(t.send(&metadata("2.0.0"), &files()).await, 409).await;
    assert_eq!(
        t.state.releases.get(&t.app, &draft.id).await.unwrap(),
        draft
    );
    assert_eq!(t.count("publication_history").await, 1);
    assert_eq!(t.count("artifacts").await, 3);
}

#[tokio::test]
async fn concurrent_identical_and_conflicting_requests_have_one_winner_and_one_event() {
    let mut t = TestApp::new().await;
    let input = metadata("1.0.0");
    let parts = files();
    let (a, b) = tokio::join!(t.send(&input, &parts), t.send(&input, &parts));
    assert_eq!(a.status().as_u16() + b.status().as_u16(), 401);
    let a_status = a.status().as_u16();
    let b_status = b.status().as_u16();
    assert_eq!(value(a, a_status).await, value(b, b_status).await);
    let input = metadata("2.0.0");
    let mut other = files();
    other[1].bytes = b"different windows package".to_vec();
    let (a, b) = tokio::join!(t.send(&input, &parts), t.send(&input, &other));
    assert_eq!(a.status().as_u16() + b.status().as_u16(), 610);
    let (winner, loser, winner_files) = if a.status() == 201 {
        (a, b, &parts)
    } else {
        (b, a, &other)
    };
    let winner = value(winner, 201).await;
    value(loser, 409).await;
    assert_eq!(value(t.send(&input, winner_files).await, 200).await, winner);
    assert_eq!(t.count("releases").await, 2);
    assert_eq!(t.count("publication_history").await, 2);
    t.restart().await;
    assert_eq!(t.count("artifacts").await, 4);
    assert_eq!(
        std::fs::read_dir(t.config.storage.directory.clone())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.path().extension().is_some_and(|e| e == "blob"))
            .count(),
        4
    );
}

#[tokio::test]
async fn invalid_parts_or_metadata_reject_the_whole_batch_and_corrected_version_is_reusable() {
    let t = TestApp::new().await;
    for (index, case) in [
        "checksum",
        "size",
        "missing",
        "extra",
        "duplicate",
        "empty",
        "target",
        "filename",
        "format",
        "roles",
        "file-signature-name",
        "signature-role",
        "version",
        "notes",
        "too-many",
    ]
    .iter()
    .enumerate()
    {
        let version = format!("1.0.{index}");
        let mut input = metadata(&version);
        let mut parts = files();
        match *case {
            "checksum" => input["artifacts"][1]["sha256"] = "0".repeat(64).into(),
            "size" => input["artifacts"][1]["size_bytes"] = 999.into(),
            "missing" => {
                parts.pop();
            }
            "extra" => parts.push(Part::file("undeclared", "extra", b"extra")),
            "duplicate" => parts.push(parts[0].clone()),
            "empty" => input["artifacts"] = json!([]),
            "target" => input["artifacts"][1]["target"] = "unknown".into(),
            "filename" => parts[1].filename = "../outside.exe".into(),
            "format" => input["artifacts"][1]["file_type"] = "msi".into(),
            "roles" => input["artifacts"][1]["file"] = "linux".into(),
            "file-signature-name" => input["artifacts"][0]["standalone_signature"] = "linux".into(),
            "signature-role" => input["artifacts"][0]["signature"] = "sig".into(),
            "version" => input["version"] = "not semver".into(),
            "notes" => input["notes"] = "x".repeat(20001).into(),
            "too-many" => {
                input["artifacts"] = Value::Array(vec![input["artifacts"][0].clone(); 65])
            }
            _ => unreachable!(),
        }
        value(
            t.send(&input, &parts).await,
            if *case == "too-many" { 413 } else { 422 },
        )
        .await;
        assert_eq!(t.count("releases").await, index as i64, "{case}");
        value(t.send(&metadata(&version), &files()).await, 201).await;
    }
    assert_eq!(t.count("publication_history").await, 15);
}

#[tokio::test]
async fn malformed_truncated_and_oversized_streams_never_publish() {
    let t = TestApp::new().await;
    let input = metadata("1.0.0");
    for bytes in [
        b"not multipart".to_vec(),
        wire(&input, &files())[..100].to_vec(),
        raw_wire(&files()),
        raw_wire(&[Part {
            name: "release".into(),
            filename: "publish.json".into(),
            bytes: b"{broken".to_vec(),
            content_type: "application/json",
        }]),
    ] {
        value(
            t.router
                .clone()
                .oneshot(t.request("product", Some(&t.token), Body::from(bytes)))
                .await
                .unwrap(),
            400,
        )
        .await;
    }
    let mut parts = files();
    parts[1].bytes = vec![1; 33 * 1024];
    value(t.send(&input, &parts).await, 413).await;
    let mut request = t.request("product", Some(&t.token), Body::empty());
    request.headers_mut().insert(
        "content-length",
        (t.config.storage.max_request_bytes + 1).into(),
    );
    value(t.router.clone().oneshot(request).await.unwrap(), 413).await;
    // No Content-Length: enforce the total budget on a streaming body as well.
    let chunk = Bytes::from(vec![b' '; t.config.storage.max_request_bytes + 1]);
    let body = Body::from_stream(futures_util::stream::iter([Ok::<_, io::Error>(chunk)]));
    value(
        t.router
            .clone()
            .oneshot(t.request("product", Some(&t.token), body))
            .await
            .unwrap(),
        413,
    )
    .await;
    let mut config = t.config.storage.clone();
    config.max_request_bytes = 300 * 1024;
    let mut state = t.state.clone();
    state.storage_config = config;
    let huge = raw_wire(&[Part {
        name: "release".into(),
        filename: "publish.json".into(),
        bytes: vec![b' '; 257 * 1024],
        content_type: "application/json",
    }]);
    value(
        http::router(state)
            .oneshot(t.request("product", Some(&t.token), Body::from(huge)))
            .await
            .unwrap(),
        413,
    )
    .await;
    assert_eq!(t.count("releases").await, 0);
    assert_eq!(t.count("artifacts").await, 0);
    value(t.send(&input, &files()).await, 201).await;
    let body = String::from_utf8(wire(&input, &files())).unwrap();
    for replacement in ["", "Content-Type: application/json; charset=utf-8\r\n"] {
        let body = body.replacen("Content-Type: application/json\r\n", replacement, 1);
        value(
            t.router
                .clone()
                .oneshot(t.request("product", Some(&t.token), Body::from(body)))
                .await
                .unwrap(),
            200,
        )
        .await;
    }
}

#[tokio::test]
async fn bearer_authentication_and_slug_binding_precede_body_reads_and_reject_http_spoofing() {
    let t = TestApp::new().await;
    for (token, slug, status) in [
        (None, "product", 401),
        (Some("vrp_invalid"), "product", 401),
        (Some(t.token.as_str()), "foreign", 403),
        (Some(t.token.as_str()), "unknown", 403),
    ] {
        let polled = Arc::new(AtomicBool::new(false));
        let observed = polled.clone();
        let body = Body::from_stream(futures_util::stream::poll_fn(move |_| {
            observed.store(true, Ordering::Relaxed);
            std::task::Poll::Ready(Some(Err::<Bytes, _>(io::Error::other("must not read"))))
        }));
        value(
            t.router
                .clone()
                .oneshot(t.request(slug, token, body))
                .await
                .unwrap(),
            status,
        )
        .await;
        assert!(!polled.load(Ordering::Relaxed));
    }
    let mut request = t.request(
        "product",
        None,
        Body::from(wire(&metadata("1.0.0"), &files())),
    );
    request.headers_mut().insert(
        "cookie",
        format!("veruna_session={}", t.session).parse().unwrap(),
    );
    value(t.router.clone().oneshot(request).await.unwrap(), 401).await;
    for peer in ["192.0.2.7:1", "127.0.0.1:1"] {
        let mut request = t.request(
            "product",
            Some(&t.token),
            Body::from(wire(&metadata("1.0.0"), &files())),
        );
        request
            .headers_mut()
            .insert("host", "apps.example.test".parse().unwrap());
        request
            .headers_mut()
            .insert("x-forwarded-proto", "https".parse().unwrap());
        request
            .extensions_mut()
            .insert(ConnectInfo(peer.parse::<SocketAddr>().unwrap()));
        let status = if peer.starts_with("127.") { 201 } else { 400 };
        value(t.router.clone().oneshot(request).await.unwrap(), status).await;
    }
    let mut request = t.request(
        "product",
        Some(&t.token),
        Body::from(wire(&metadata("1.0.0"), &files())),
    );
    request
        .headers_mut()
        .insert("host", "127.0.0.1:8080".parse().unwrap());
    request
        .extensions_mut()
        .insert(ConnectInfo("192.0.2.7:1".parse::<SocketAddr>().unwrap()));
    value(t.router.clone().oneshot(request).await.unwrap(), 403).await;
    let mut request = t.request("product", Some(&t.token), Body::empty());
    request
        .headers_mut()
        .append("authorization", "Bearer another".parse().unwrap());
    value(t.router.clone().oneshot(request).await.unwrap(), 401).await;
    // Fixed public URLs must not turn an untrusted HTTP connection into TLS.
    let mut fixed = t.state.clone();
    fixed.auth_config.public_origin = "https://apps.example.test".into();
    fixed.distribution.config.public_base_url = "https://apps.example.test".into();
    let mut request = t.request("product", Some(&t.token), Body::empty());
    request
        .headers_mut()
        .insert("host", "apps.example.test".parse().unwrap());
    request
        .headers_mut()
        .insert("x-forwarded-proto", "https".parse().unwrap());
    request
        .extensions_mut()
        .insert(ConnectInfo("192.0.2.7:1".parse::<SocketAddr>().unwrap()));
    value(http::router(fixed).oneshot(request).await.unwrap(), 400).await;
}

#[tokio::test]
async fn private_archived_and_prerelease_keep_existing_distribution_rules() {
    let t = TestApp::new().await;
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
    let first = value(t.send(&metadata("1.0.0"), &files()).await, 201).await;
    assert_eq!(
        t.state.applications.get(&t.app).await.unwrap().visibility,
        Visibility::Private
    );
    let r = t
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/product/manifest.json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
    t.state
        .applications
        .set_status(&t.actor, &t.app, Status::Archived)
        .await
        .unwrap();
    value(t.send(&metadata("2.0.0"), &files()).await, 409).await;
    assert_eq!(t.count("releases").await, 1);
    t.state
        .applications
        .set_status(&t.actor, &t.app, Status::Active)
        .await
        .unwrap();
    t.state
        .applications
        .update(
            &t.actor,
            &t.app,
            ApplicationChanges {
                visibility: Some(Visibility::Public),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let mut preview = files();
    preview[0].bytes = b"preview executable".to_vec();
    value(t.send(&metadata("2.0.0-rc.1"), &preview).await, 201).await;
    t.download(
        "/product/latest?target=linux-x86_64&type=executable",
        CONTENT,
    )
    .await;
    let published = t
        .state
        .releases
        .get(&t.app, first["release_id"].as_str().unwrap())
        .await
        .unwrap();
    assert_eq!(published.version.to_string(), "1.0.0");
}

fn tauri_signature() -> Vec<u8> {
    let mut packet = vec![0u8; 74];
    packet[..2].copy_from_slice(b"ED");
    STANDARD
        .encode(format!(
            "untrusted comment: test\n{}\ntrusted comment: test\n{}\n",
            STANDARD.encode(packet),
            STANDARD.encode([0u8; 64])
        ))
        .into_bytes()
}
#[tokio::test]
async fn tauri_shared_windows_part_has_two_roles_unique_keys_and_requires_readable_sig() {
    let t = TestApp::new().await;
    let input = json!({"version":"1.0.0","notes":NOTES,"artifacts":[
        {"file":"package","target":"windows-x86_64","file_type":"exe","distribution_role":"installer"},
        {"file":"package","target":"windows-x86_64","file_type":"exe","distribution_role":"updater","signature":"sig"}]});
    let parts = vec![
        Part::file("sig", "package.exe.sig", &tauri_signature()),
        Part::file("package", "package.exe", CONTENT),
    ];
    let mut missing = input.clone();
    missing["artifacts"][1]
        .as_object_mut()
        .unwrap()
        .remove("signature");
    value(t.send(&missing, &parts).await, 422).await;
    value(t.send(&input, &parts[1..]).await, 422).await;
    for bytes in [
        b"".to_vec(),
        vec![255],
        b"not a signature".to_vec(),
        b"file.sig".to_vec(),
    ] {
        let mut bad = parts.clone();
        bad[0].bytes = bytes;
        value(t.send(&input, &bad).await, 422).await;
    }
    let mut bad = parts.clone();
    bad[0].bytes = vec![b'a'; 16 * 1024 + 1];
    value(t.send(&input, &bad).await, 413).await;
    let first = value(t.send(&input, &parts).await, 201).await;
    let artifacts = t
        .state
        .artifacts
        .list(&t.app, first["release_id"].as_str().unwrap())
        .await
        .unwrap();
    assert_eq!(artifacts.len(), 2);
    assert_ne!(artifacts[0].storage_key, artifacts[1].storage_key);
    for a in artifacts {
        assert_eq!(a.sha256, hash(CONTENT));
        t.download(
            &format!(
                "/product/releases/{}/artifacts/{}/download",
                first["version"].as_str().unwrap(),
                a.id
            ),
            CONTENT,
        )
        .await;
    }
    let response = value(
        t.router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/updates/tauri/product/windows/x86_64/0.0.0")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
        200,
    )
    .await;
    assert_eq!(response["version"], "1.0.0");
    assert_eq!(
        response["signature"],
        String::from_utf8(tauri_signature()).unwrap()
    );
    assert_eq!(value(t.send(&input, &parts).await, 200).await, first);
    assert_eq!(t.count("publication_history").await, 1);
}

// RFC 8032 public test vector, never a production key.
const PUBLIC_KEY: &str = "11qYAYKxCrfVS_7TyWQHOg7hcvPapiMlrwIaaPcHURo";
const SEED: [u8; 32] = [
    0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec, 0x2c, 0xc4,
    0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03, 0x1c, 0xae, 0x7f, 0x60,
];
fn jws(product: &str, version: &str, seed: &[u8; 32]) -> String {
    let payload = json!({"schema_version":1,"application_id":product,"version":version,"os":"linux","arch":"x86_64","size_bytes":CONTENT.len(),"sha256":hash(CONTENT)});
    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::EdDSA);
    header.typ = Some(veruna_releases::domain::standalone::MEDIA_TYPE.into());
    let mut der = vec![
        0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04,
        0x20,
    ];
    der.extend_from_slice(seed);
    jsonwebtoken::encode(
        &header,
        &payload,
        &jsonwebtoken::EncodingKey::from_ed_der(&der),
    )
    .unwrap()
}
#[tokio::test]
async fn standalone_uses_existing_jws_verification_and_bad_second_artifact_rejects_all() {
    let t = TestApp::new().await;
    let product = uuid::Uuid::new_v4().to_string();
    t.state
        .applications
        .set_standalone_settings(
            &t.actor,
            &t.app,
            Some(Some(&product)),
            Some(Some(PUBLIC_KEY)),
        )
        .await
        .unwrap();
    let mut input = metadata("1.0.0");
    input["artifacts"][0]["standalone_signature"] = "jws".into();
    for signature in [
        jws(&product, "1.0.0", &[7u8; 32]),
        jws(&uuid::Uuid::new_v4().to_string(), "1.0.0", &SEED),
        jws(&product, "2.0.0", &SEED),
    ] {
        let mut parts = files();
        parts.push(Part::file("jws", "product.jws", signature.as_bytes()));
        value(t.send(&input, &parts).await, 422).await;
        assert_eq!(t.count("releases").await, 0);
    }
    let signature = jws(&product, "1.0.0", &SEED);
    let mut parts = files();
    parts.insert(0, Part::file("jws", "product.jws", signature.as_bytes()));
    let mut bad = parts.clone();
    bad[1].bytes = b"wrong executable bytes".to_vec();
    value(t.send(&input, &bad).await, 422).await;
    let first = value(t.send(&input, &parts).await, 201).await;
    assert_eq!(value(t.send(&input, &parts).await, 200).await, first);
    let a = t
        .state
        .artifacts
        .list(&t.app, first["release_id"].as_str().unwrap())
        .await
        .unwrap();
    assert_eq!(
        a.iter()
            .find(|a| a.target == "linux-x86_64")
            .unwrap()
            .standalone_signature
            .as_deref(),
        Some(signature.as_str())
    );
    assert!(
        t.state
            .applications
            .standalone_settings(&t.app)
            .await
            .unwrap()
            .1
    );
}

async fn streaming(
    t: &TestApp,
    version: &str,
) -> (
    tokio::sync::mpsc::Sender<Result<Bytes, io::Error>>,
    tokio::task::JoinHandle<Response>,
    Vec<u8>,
) {
    let all = wire(&metadata(version), &files());
    let split = all.len() - b"windows package\r\n--BATCH--\r\n".len();
    let (sender, receiver) = tokio::sync::mpsc::channel(2);
    let polled = Arc::new(AtomicBool::new(false));
    let observed = polled.clone();
    let stream =
        futures_util::stream::unfold((receiver, observed), |(mut rx, observed)| async move {
            rx.recv().await.map(|bytes| {
                observed.store(true, Ordering::Relaxed);
                (bytes, (rx, observed))
            })
        });
    let request = t.request("product", Some(&t.token), Body::from_stream(stream));
    let router = t.router.clone();
    let task = tokio::spawn(async move { router.oneshot(request).await.unwrap() });
    sender
        .send(Ok(Bytes::copy_from_slice(&all[..split])))
        .await
        .unwrap();
    for _ in 0..100 {
        if polled.load(Ordering::Relaxed) {
            return (sender, task, all[split..].to_vec());
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("stream did not start");
}
#[tokio::test]
async fn revoked_or_expired_during_upload_and_changed_settings_cannot_commit() {
    let t = TestApp::new().await;
    let (sender, task, tail) = streaming(&t, "1.0.0").await;
    let token_id: String = sqlx::query_scalar("SELECT id FROM api_tokens WHERE token_hash=?")
        .bind(secrets::digest(&t.token))
        .fetch_one(&t.state.database)
        .await
        .unwrap();
    t.state
        .api_tokens
        .revoke(&t.actor, &t.app, &token_id)
        .await
        .unwrap();
    sender.send(Ok(Bytes::from(tail))).await.unwrap();
    drop(sender);
    value(task.await.unwrap(), 401).await;
    assert_eq!(t.count("releases").await, 0);
    let new = t
        .state
        .api_tokens
        .create(&t.actor, &t.app, "New CI", None)
        .await
        .unwrap();
    let r = t
        .router
        .clone()
        .oneshot(t.request(
            "product",
            Some(&new.secret),
            Body::from(wire(&metadata("1.0.0"), &files())),
        ))
        .await
        .unwrap();
    value(r, 201).await;
    let mut t = TestApp::new().await;
    let expires = secrets::now() + 2;
    let new = t
        .state
        .api_tokens
        .create(&t.actor, &t.app, "Expiring", Some(expires))
        .await
        .unwrap();
    t.token = new.secret;
    let (sender, task, tail) = streaming(&t, "1.0.0").await;
    // Keep upload alive until the token's expiry, including the final idle period.
    while secrets::now() < expires {
        sender.send(Ok(Bytes::from_static(b"x"))).await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    sender.send(Ok(Bytes::from(tail))).await.unwrap();
    drop(sender);
    value(task.await.unwrap(), 401).await;
    value(t.send(&metadata("1.0.0"), &files()).await, 401).await;
    assert_eq!(t.count("releases").await, 0);
}

#[tokio::test]
async fn settings_and_archive_races_idle_timeout_and_cancelled_last_file_preserve_previous_latest()
{
    let mut t = TestApp::new().await;
    value(t.send(&metadata("1.0.0"), &files()).await, 201).await;
    let (sender, task, tail) = streaming(&t, "2.0.0").await;
    let product = uuid::Uuid::new_v4().to_string();
    t.state
        .applications
        .set_standalone_settings(
            &t.actor,
            &t.app,
            Some(Some(&product)),
            Some(Some(PUBLIC_KEY)),
        )
        .await
        .unwrap();
    sender.send(Ok(Bytes::from(tail))).await.unwrap();
    drop(sender);
    value(task.await.unwrap(), 409).await;
    let (sender, task, tail) = streaming(&t, "2.0.0").await;
    t.state
        .applications
        .set_status(&t.actor, &t.app, Status::Archived)
        .await
        .unwrap();
    sender.send(Ok(Bytes::from(tail))).await.unwrap();
    drop(sender);
    value(task.await.unwrap(), 409).await;
    t.state
        .applications
        .set_status(&t.actor, &t.app, Status::Active)
        .await
        .unwrap();
    let (sender, task, _) = streaming(&t, "2.0.0").await;
    drop(sender);
    value(task.await.unwrap(), 400).await;
    let (sender, task, _) = streaming(&t, "2.0.0").await;
    value(task.await.unwrap(), 408).await;
    drop(sender);
    let (sender, task, _) = streaming(&t, "2.0.0").await;
    task.abort();
    drop(sender);
    assert!(task.await.is_err());
    assert_eq!(t.count("releases").await, 1);
    assert_eq!(t.count("publication_history").await, 1);
    t.download(
        "/product/latest?target=linux-x86_64&type=executable",
        CONTENT,
    )
    .await;
    t.restart().await;
    value(t.send(&metadata("2.0.0"), &files()).await, 201).await;
}

#[tokio::test]
async fn final_transaction_failure_rolls_back_every_record_and_allows_retry() {
    let mut t = TestApp::new().await;
    value(t.send(&metadata("1.0.0"), &files()).await, 201).await;
    sqlx::query("CREATE TRIGGER fail_api_publication BEFORE INSERT ON api_publications BEGIN SELECT RAISE(ABORT,'test failure'); END").execute(&t.state.database).await.unwrap();
    value(t.send(&metadata("2.0.0"), &files()).await, 500).await;
    assert_eq!(t.count("releases").await, 1);
    assert_eq!(t.count("artifacts").await, 2);
    assert_eq!(t.count("publication_history").await, 1);
    t.download(
        "/product/latest?target=linux-x86_64&type=executable",
        CONTENT,
    )
    .await;
    sqlx::query("DROP TRIGGER fail_api_publication")
        .execute(&t.state.database)
        .await
        .unwrap();
    t.restart().await;
    value(t.send(&metadata("2.0.0"), &files()).await, 201).await;
}

#[tokio::test]
async fn concurrency_and_total_batch_limits_and_lost_response_retry_are_enforced() {
    let mut t = TestApp::new().await;
    t.config.storage.max_concurrent_uploads = 1;
    t.state = application_state(t.state.database.clone(), &t.config)
        .await
        .unwrap();
    t.router = http::router(t.state.clone());
    let (sender, task, _) = streaming(&t, "1.0.0").await;
    value(t.send(&metadata("2.0.0"), &files()).await, 503).await;
    drop(sender);
    value(task.await.unwrap(), 400).await;
    assert_eq!(t.count("releases").await, 0);
    let targets = [
        "linux-x86_64",
        "linux-aarch64",
        "windows-x86_64",
        "darwin-x86_64",
        "darwin-aarch64",
    ];
    let mut input = json!({"version":"1.0.0","notes":NOTES,"artifacts":[]});
    let parts: Vec<_> = targets.into_iter().enumerate().map(|(i,target)| {
        let name = format!("file_{i}");
        input["artifacts"].as_array_mut().unwrap().push(json!({"file":name,"target":target,"file_type":"bin","distribution_role":"executable"}));
        Part::file(&name,&format!("product-{target}"),&vec![1;28*1024])
    }).collect();
    let bytes = wire(&input, &parts);
    let chunks: Vec<_> = bytes
        .chunks(4096)
        .map(|bytes| Ok::<_, io::Error>(Bytes::copy_from_slice(bytes)))
        .collect();
    let response = t
        .router
        .clone()
        .oneshot(t.request(
            "product",
            Some(&t.token),
            Body::from_stream(futures_util::stream::iter(chunks)),
        ))
        .await
        .unwrap();
    value(response, 413).await;
    assert_eq!(t.count("releases").await, 0);
    // Lose the response body after the server has committed, then retry.
    // The inclusive per-file limit also exceeds the router's small JSON default.
    let mut accepted = files();
    accepted[0].bytes = vec![b'a'; t.config.storage.max_file_bytes as usize];
    let response = t.send(&metadata("1.0.0"), &accepted).await;
    assert_eq!(response.status(), 201);
    drop(response);
    let original: (String, i64) = sqlx::query_as("SELECT id,published_at FROM releases")
        .fetch_one(&t.state.database)
        .await
        .unwrap();
    let retry = value(t.send(&metadata("1.0.0"), &accepted).await, 200).await;
    assert_eq!(retry["release_id"], original.0);
    assert_eq!(retry["published_at"], original.1);
    assert_eq!(t.count("publication_history").await, 1);
    let (a, b) = tokio::join!(
        t.state.api_tokens.create(&t.actor, &t.app, "A", None),
        t.state.api_tokens.create(&t.actor, &t.app, "B", None)
    );
    assert_ne!(a.is_ok(), b.is_ok());
    assert!(matches!(
        a.err().or_else(|| b.err()),
        Some(veruna_releases::services::error::ServiceError::ApiTokenLimit)
    ));
    assert_eq!(t.count("api_tokens").await, 2);
}

#[tokio::test]
async fn migration_preserves_user_history_and_enforces_one_author_and_retention() {
    let dir = tempdir().unwrap();
    let mut config = AppConfig::default();
    config.database.path = dir.path().join("legacy.sqlite");
    config.storage.directory = dir.path().join("objects");
    config.storage.icons_directory = dir.path().join("icons");
    let mut old = sqlx::migrate!("./migrations");
    old.migrations = old
        .iter()
        .filter(|m| m.version < 16)
        .cloned()
        .collect::<Vec<_>>()
        .into();
    let pool = sqlx::SqlitePool::connect_with(
        sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&config.database.path)
            .create_if_missing(true)
            .foreign_keys(true),
    )
    .await
    .unwrap();
    old.run(&pool).await.unwrap();
    let app = uuid::Uuid::new_v4().to_string();
    let release = uuid::Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO users(id,username,password_hash,role,status,created_at,updated_at) VALUES('legacy-user','legacy',NULL,'operator','invited',1,1)").execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO applications(id,slug,name,name_search,created_at,updated_at) VALUES(?,'legacy','Legacy','legacy',1,1)").bind(&app).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO releases(id,application_id,version,version_key,created_at,updated_at) VALUES(?,?,'1.0.0','1.0.0',1,1)").bind(&release).bind(&app).execute(&pool).await.unwrap();
    sqlx::query("UPDATE releases SET status='published',published_at=2 WHERE id=?")
        .bind(&release)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO publication_history(id,application_id,release_id,user_id,created_at) VALUES(7,?,?,'legacy-user',2)").bind(&app).bind(&release).execute(&pool).await.unwrap();
    pool.close().await;
    let pool = initialize(&config).await.unwrap();
    let row: (i64, String, String, String, Option<String>, i64) = sqlx::query_as(
        "SELECT id,application_id,release_id,user_id,token_id,created_at FROM publication_history",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        row,
        (
            7,
            app.clone(),
            release.clone(),
            "legacy-user".into(),
            None,
            2
        )
    );
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&pool)
            .await
            .unwrap()
            .is_empty()
    );
    for sql in [
        "UPDATE publication_history SET user_id=NULL",
        "DELETE FROM publication_history",
    ] {
        assert!(sqlx::query(sql).execute(&pool).await.is_err());
    }
    // Remove the immutable guard in this isolated test to exercise CHECK itself.
    sqlx::query("DROP TRIGGER publication_history_no_update")
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        sqlx::query("UPDATE publication_history SET user_id=NULL")
            .execute(&pool)
            .await
            .is_err()
    );
    let token = uuid::Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO api_tokens(id,application_id,token_hash,name,created_at,created_by) VALUES(?,? ,?,'CI',1,'legacy-user')")
        .bind(&token).bind(&app).bind("a".repeat(64)).execute(&pool).await.unwrap();
    assert!(
        sqlx::query("UPDATE publication_history SET token_id=?")
            .bind(&token)
            .execute(&pool)
            .await
            .is_err()
    );
    sqlx::query("UPDATE publication_history SET user_id=NULL,token_id=?")
        .bind(&token)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        sqlx::query("DELETE FROM api_tokens")
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE api_tokens SET token_hash=?")
            .bind("b".repeat(64))
            .execute(&pool)
            .await
            .is_err()
    );
}
