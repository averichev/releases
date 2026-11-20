use axum::{Router, body::Body, http::Request, response::Response};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use http_body_util::BodyExt;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tempfile::{TempDir, tempdir};
use tower::ServiceExt;
use veruna_releases::{
    application_state,
    config::AppConfig,
    domain::{
        application::{ApplicationChanges, NewApplication, Status, Visibility},
        artifact::{Artifact, UploadMetadata},
        release::{Release, ReleaseChanges, ReleaseInput},
        standalone,
    },
    http::{self, AppState},
    infrastructure::secrets,
    initialize,
    services::{
        artifact::ArtifactUpload, error::ServiceError, publishing::PublicationCommand,
        sessions::Identity,
    },
};

// RFC 8037 / RFC 8032 test vector, never a production credential.
const PUBLIC_KEY: &str = "11qYAYKxCrfVS_7TyWQHOg7hcvPapiMlrwIaaPcHURo";
const SEED: [u8; 32] = [
    0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec, 0x2c, 0xc4,
    0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03, 0x1c, 0xae, 0x7f, 0x60,
];
const CONTENT: &[u8] = b"standalone-test-executable";
fn sign_header(payload: &Value, header: &Header, seed: &[u8; 32]) -> String {
    // PKCS#8 Ed25519 seed envelope, per RFC 8410.
    let mut der = vec![
        0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04,
        0x20,
    ];
    der.extend_from_slice(seed);
    jsonwebtoken::encode(header, payload, &EncodingKey::from_ed_der(&der)).unwrap()
}
fn sign(payload: &Value) -> String {
    let mut header = Header::new(Algorithm::EdDSA);
    header.typ = Some(standalone::MEDIA_TYPE.into());
    sign_header(payload, &header, &SEED)
}
fn metadata(app: &str, version: &str, target: &str) -> Value {
    let (os, arch) = target.split_once('-').unwrap();
    json!({"schema_version":1,"application_id":app,"version":version,"os":os,"arch":arch,"size_bytes":CONTENT.len(),"sha256":format!("{:x}",Sha256::digest(CONTENT))})
}
struct TestApp {
    _dir: TempDir,
    config: AppConfig,
    state: AppState,
    router: Router,
    actor: Identity,
    token: String,
    app: String,
}
impl TestApp {
    async fn new() -> Self {
        let dir = tempdir().unwrap();
        let mut config = AppConfig::default();
        config.database.path = dir.path().join("db.sqlite");
        config.storage.directory = dir.path().join("objects");
        config.storage.icons_directory = dir.path().join("icons");
        // Automatic request origin must propagate to the new endpoint too.
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
        state
            .applications
            .set_standalone_key(&actor, &app, Some(PUBLIC_KEY))
            .await
            .unwrap();
        let router = http::router(state.clone());
        Self {
            _dir: dir,
            config,
            state,
            router,
            actor,
            token: login.token,
            app,
        }
    }
    fn base(&self) -> String {
        format!("/api/v1/admin/applications/{}", self.app)
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
                    release_notes: String::new(),
                },
            )
            .await
            .unwrap()
    }
    async fn begin(&self, r: &Release, target: &str) -> ArtifactUpload {
        self.state
            .artifacts
            .begin(
                &self.actor,
                &self.app,
                &r.id,
                UploadMetadata {
                    filename: if target.starts_with("windows-") {
                        "product.exe".into()
                    } else {
                        "product".into()
                    },
                    target: target.into(),
                    file_type: Some(
                        if target.starts_with("windows-") {
                            "exe"
                        } else {
                            "bin"
                        }
                        .into(),
                    ),
                    distribution_role: None,
                },
            )
            .await
            .unwrap()
    }
    async fn upload(&self, r: &Release, target: &str, signed: bool) -> Artifact {
        let mut upload = self.begin(r, target).await;
        upload.write(CONTENT).await.unwrap();
        if signed {
            upload
                .set_standalone_signature(
                    sign(&metadata(&self.app, &r.version.to_string(), target)).as_bytes(),
                )
                .unwrap();
        }
        self.state
            .artifacts
            .complete(&self.actor, upload)
            .await
            .unwrap()
    }
    async fn publish(&self, r: &Release) -> Result<Release, ServiceError> {
        let r = self.state.releases.get(&self.app, &r.id).await.unwrap();
        self.state
            .publishing
            .publish(
                &self.actor,
                &self.app,
                &r.id,
                PublicationCommand {
                    expected_release_revision: r.revision,
                },
            )
            .await
    }
    async fn get(&self, path: &str) -> Response {
        self.router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(path)
                    .header("host", "127.0.0.1:8080")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
    }
    async fn update(&self, target: &str, current: &str) -> Response {
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("current_version", current)
            .finish();
        self.get(&format!(
            "/api/v1/updates/standalone/product/{target}?{query}"
        ))
        .await
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
            .header("host", "127.0.0.1:8080")
            .header("origin", "http://127.0.0.1:8080")
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
    async fn multipart(
        &self,
        r: &Release,
        target: &str,
        jws: Option<&str>,
        truncated: bool,
    ) -> Response {
        let filename = if target.starts_with("windows-") {
            "product.exe"
        } else {
            "product.bin"
        };
        let mut form = format!(
            "--BOUNDARY\r\nContent-Disposition: form-data; name=\"target\"\r\n\r\n{target}\r\n--BOUNDARY\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\nContent-Type: application/octet-stream\r\n\r\n{}\r\n",
            std::str::from_utf8(CONTENT).unwrap()
        );
        if let Some(jws) = jws {
            form.push_str(&format!("--BOUNDARY\r\nContent-Disposition: form-data; name=\"standalone_signature\"; filename=\"product.jws\"\r\n\r\n{jws}\r\n"));
        }
        if !truncated {
            form.push_str("--BOUNDARY--\r\n");
        }
        self.router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("{}/releases/{}/artifacts", self.base(), r.id))
                    .header("host", "127.0.0.1:8080")
                    .header("origin", "http://127.0.0.1:8080")
                    .header("x-veruna-request", "1")
                    .header("cookie", format!("veruna_session={}", self.token))
                    .header("x-csrf-token", secrets::csrf(&self.token))
                    .header("content-type", "multipart/form-data; boundary=BOUNDARY")
                    .body(Body::from(form))
                    .unwrap(),
            )
            .await
            .unwrap()
    }
    async fn assert_storage_count(&self, expected: usize) {
        for _ in 0..100 {
            let objects = std::fs::read_dir(&self.config.storage.directory)
                .unwrap()
                .count()
                - 1; // .tmp
            let temporary = std::fs::read_dir(self.config.storage.temporary_directory())
                .unwrap()
                .count();
            if objects == expected && temporary == 0 {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("Upload compensation did not reclaim unreferenced files");
    }
}
async fn body(r: Response) -> Value {
    serde_json::from_slice(&r.into_body().collect().await.unwrap().to_bytes()).unwrap()
}

#[tokio::test]
async fn three_signed_installers_sparse_platforms_semver_and_manifest_remain_compatible() {
    let t = TestApp::new().await;
    let old = t.create("0.5.0+ci.7").await;
    for target in ["windows-x86_64", "darwin-aarch64", "linux-x86_64"] {
        let r = t
            .multipart(
                &old,
                target,
                Some(&sign(&metadata(&t.app, "0.5.0+ci.7", target))),
                false,
            )
            .await;
        assert_eq!(r.status(), 201);
        let dto = body(r).await;
        assert_eq!(dto["distribution_role"], "installer");
        assert_eq!(dto["standalone_signature_present"], true);
    }
    t.publish(&old).await.unwrap();
    let newer = t.create("0.6.0").await;
    t.upload(&newer, "darwin-aarch64", true).await;
    t.publish(&newer).await.unwrap();
    let unsigned = t.create("9.0.0").await;
    let unsigned_artifact = t.upload(&unsigned, "windows-x86_64", false).await;
    t.publish(&unsigned).await.unwrap();
    let unsigned_path =
        t.state
            .distribution
            .artifact_path("product", &unsigned, &unsigned_artifact);
    let manual = t.get(&unsigned_path).await;
    assert_eq!(manual.status(), 200);
    assert!(manual.headers().contains_key("content-disposition"));
    assert_eq!(
        manual
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .as_ref(),
        CONTENT
    );
    let shortcut = t.get("/product/latest/windows-x86_64").await;
    assert_eq!(shortcut.status(), 307);
    assert_eq!(shortcut.headers()["location"], unsigned_path);
    let originals = t.state.artifacts.list(&t.app, &old.id).await.unwrap();
    assert_eq!(originals.len(), 3);
    assert!(
        originals
            .iter()
            .all(|a| a.distribution_role == "installer" && a.updater_signature.is_none())
    );
    let pre = t.create("10.0.0-rc.1").await;
    t.upload(&pre, "windows-x86_64", true).await;
    t.publish(&pre).await.unwrap();
    let draft = t.create("11.0.0").await;
    t.upload(&draft, "windows-x86_64", true).await;
    for (target, version) in [
        ("windows-x86_64", "0.5.0+ci.7"),
        ("darwin-aarch64", "0.6.0"),
        ("linux-x86_64", "0.5.0+ci.7"),
    ] {
        let response = t.update(target, "0.4.0").await;
        assert_eq!(response.status(), 200);
        assert_eq!(response.headers()["cache-control"], "no-store");
        assert_eq!(response.headers()["access-control-allow-origin"], "*");
        let dto = body(response).await;
        assert_eq!(dto["version"], version);
        let url = url::Url::parse(dto["url"].as_str().unwrap()).unwrap();
        assert_eq!(url.origin().ascii_serialization(), "http://127.0.0.1:8080");
        let bytes = t
            .get(url.path())
            .await
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes();
        assert_eq!(bytes.as_ref(), CONTENT);
        let signature = dto["signed_metadata"].as_str().unwrap();
        assert_eq!(signature, sign(&metadata(&t.app, version, target)));
    }
    for version in ["0.5.0", "0.5.0+other", "0.6.0", "99.0.0"] {
        assert_eq!(t.update("windows-x86_64", version).await.status(), 204);
    }
    assert_eq!(t.update("windows-aarch64", "0.1.0").await.status(), 204);
    // Existing installer latest semantics intentionally continue to include unsigned files.
    let manifest = body(t.get("/product/manifest.json").await).await;
    assert_eq!(manifest["schema_version"], 2);
    assert_eq!(
        manifest["latest_by_target"]["windows-x86_64"]["version"],
        "9.0.0"
    );
    assert_eq!(
        manifest["latest_by_target"]["darwin-aarch64"]["version"],
        "0.6.0"
    );
    assert_eq!(
        manifest["latest_by_target"]["linux-x86_64"]["artifacts"][0]["purpose"],
        "installer"
    );
    assert!(
        manifest["releases"][0]["artifacts"][0]
            .get("standalone_signature")
            .is_none()
    );
    assert_eq!(
        t.get("/api/v1/updates/tauri/product/windows/x86_64/0.1.0")
            .await
            .status(),
        204
    );
    t.assert_storage_count(7).await;
}

#[tokio::test]
async fn signed_claims_and_jose_profile_are_verified_and_invalid_uploads_are_atomic() {
    let t = TestApp::new().await;
    let r = t.create("0.5.0").await;
    let target = "windows-x86_64";
    let valid = metadata(&t.app, "0.5.0", target);
    for (field, value) in [
        ("schema_version", json!(2)),
        ("application_id", json!(uuid::Uuid::new_v4().to_string())),
        ("version", json!("0.6.0")),
        ("os", json!("linux")),
        ("arch", json!("aarch64")),
        ("size_bytes", json!(1)),
        ("sha256", json!("0".repeat(64))),
        ("extra", json!(true)),
    ] {
        let mut m = valid.clone();
        m[field] = value;
        let response = t.multipart(&r, target, Some(&sign(&m)), false).await;
        assert_eq!(response.status(), 422, "claim: {field}");
    }
    let mut header = Header::new(Algorithm::EdDSA);
    header.typ = Some(standalone::MEDIA_TYPE.into());
    let wrong_key = sign_header(&valid, &header, &[42; 32]);
    assert_eq!(
        t.multipart(&r, target, Some(&wrong_key), false)
            .await
            .status(),
        422
    );
    let mut symmetric_header = Header::new(Algorithm::HS256);
    symmetric_header.typ = Some(standalone::MEDIA_TYPE.into());
    let algorithm_confusion = jsonwebtoken::encode(
        &symmetric_header,
        &valid,
        &EncodingKey::from_secret(PUBLIC_KEY.as_bytes()).unwrap(),
    )
    .unwrap();
    assert_eq!(
        t.multipart(&r, target, Some(&algorithm_confusion), false)
            .await
            .status(),
        422
    );
    let mut pieces: Vec<String> = sign(&valid).split('.').map(str::to_owned).collect();
    let mut tampered = valid.clone();
    tampered["version"] = json!("0.9.0");
    pieces[1] = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&tampered).unwrap());
    assert_eq!(
        t.multipart(&r, target, Some(&pieces.join(".")), false)
            .await
            .status(),
        422
    );
    header.typ = Some("JWT".into());
    assert_eq!(
        t.multipart(
            &r,
            target,
            Some(&sign_header(&valid, &header, &SEED)),
            false
        )
        .await
        .status(),
        422
    );
    header.typ = Some(standalone::MEDIA_TYPE.into());
    header.extras.insert("b64", false);
    assert_eq!(
        t.multipart(
            &r,
            target,
            Some(&sign_header(&valid, &header, &SEED)),
            false
        )
        .await
        .status(),
        422
    );
    for invalid in ["", "not-a-signature", "e30.e30."] {
        assert_eq!(
            t.multipart(&r, target, Some(invalid), false).await.status(),
            422
        );
    }
    // A key must exist before marking any installer for automatic updates.
    t.state
        .applications
        .set_standalone_key(&t.actor, &t.app, None)
        .await
        .unwrap();
    assert_eq!(
        t.multipart(&r, target, Some(&sign(&valid)), false)
            .await
            .status(),
        422
    );
    assert!(
        t.state
            .artifacts
            .list(&t.app, &r.id)
            .await
            .unwrap()
            .is_empty()
    );
    t.assert_storage_count(0).await;
    // An ordinary unsigned installer remains valid, but is never an update.
    t.upload(&r, target, false).await;
    t.publish(&r).await.unwrap();
    assert_eq!(t.update(target, "0.1.0").await.status(), 204);
}

#[tokio::test]
async fn api_parameters_privacy_admin_csrf_and_live_authorization() {
    let t = TestApp::new().await;
    for suffix in [
        "",
        "?current_version=bad",
        "?current_version=0.1.0&url=/etc/passwd",
        "?current_version=0.1.0&current_version=0.2.0",
    ] {
        let status = t
            .get(&format!(
                "/api/v1/updates/standalone/product/windows-x86_64{suffix}"
            ))
            .await
            .status();
        assert!(status == 400 || status == 422);
    }
    for target in ["any", "darwin-universal", "macos-arm64", "unknown"] {
        assert_eq!(t.update(target, "0.1.0").await.status(), 422);
    }
    assert_eq!(
        t.get("/api/v1/updates/standalone/missing/windows-x86_64?current_version=0.1.0")
            .await
            .status(),
        404
    );
    let settings = format!("{}/integrations/standalone", t.base());
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
    assert!(matches!(
        t.state
            .applications
            .set_standalone_key(&t.actor, &other.id, Some(PUBLIC_KEY))
            .await,
        Err(ServiceError::StandaloneKeyConflict)
    ));
    let r = t.create("0.5.0").await;
    let a = t.upload(&r, "windows-x86_64", false).await;
    let artifact = format!(
        "{}/releases/{}/artifacts/{}/standalone",
        t.base(),
        r.id,
        a.id
    );
    for (path, payload) in [
        (&settings, json!({"public_key":PUBLIC_KEY})),
        (
            &artifact,
            json!({"signed_metadata":sign(&metadata(&t.app,"0.5.0","windows-x86_64"))}),
        ),
    ] {
        assert_eq!(
            t.admin("PUT", path, payload.clone(), None, true)
                .await
                .status(),
            401
        );
        assert_eq!(
            t.admin("PUT", path, payload.clone(), Some(&t.token), false)
                .await
                .status(),
            403
        );
    }
    t.state
        .users
        .create(
            &t.actor,
            "operator",
            veruna_releases::domain::user::Role::Operator,
            Some("test-password-123".into()),
        )
        .await
        .unwrap();
    let login = t
        .state
        .authentication
        .login("operator", "test-password-123".into(), None)
        .await
        .unwrap();
    for path in [&settings, &artifact] {
        assert_eq!(
            t.admin("PUT", path, json!({}), Some(&login.token), true)
                .await
                .status(),
            403
        );
    }
    assert_eq!(
        t.admin("GET", &settings, Value::Null, Some(&login.token), true)
            .await
            .status(),
        200
    );
    for key in ["private", "A".repeat(86).as_str(), "A".repeat(43).as_str()] {
        assert_eq!(
            t.admin(
                "PUT",
                &settings,
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
            &artifact,
            json!({"signed_metadata":""}),
            Some(&t.token),
            true
        )
        .await
        .status(),
        422
    );
    assert_eq!(
        t.admin(
            "PUT",
            &artifact,
            json!({"signed_metadata":sign(&metadata(&t.app,"0.5.0","windows-x86_64"))}),
            Some(&t.token),
            true
        )
        .await
        .status(),
        200
    );
    t.publish(&r).await.unwrap();
    assert_eq!(
        t.admin(
            "PUT",
            &settings,
            json!({"public_key":null}),
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
            &artifact,
            json!({"signed_metadata":null}),
            Some(&t.token),
            true
        )
        .await
        .status(),
        409
    );
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
    assert_eq!(t.update("windows-x86_64", "0.1.0").await.status(), 404);
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
    t.state
        .applications
        .set_status(&t.actor, &t.app, Status::Archived)
        .await
        .unwrap();
    assert_eq!(t.update("windows-x86_64", "0.1.0").await.status(), 404);
    t.state
        .users
        .revoke(&t.actor, &t.actor.user.id)
        .await
        .unwrap();
    assert!(matches!(
        t.state
            .applications
            .set_standalone_key(&t.actor, &t.app, Some(PUBLIC_KEY))
            .await,
        Err(ServiceError::Unauthorized)
    ));
    assert_eq!(
        t.admin(
            "PUT",
            &settings,
            json!({"public_key":PUBLIC_KEY}),
            Some(&t.token),
            true
        )
        .await
        .status(),
        401
    );
}

#[tokio::test]
async fn integrity_publication_rollback_and_missing_file_block_instead_of_falling_back() {
    let t = TestApp::new().await;
    let old = t.create("0.4.0").await;
    t.upload(&old, "windows-x86_64", true).await;
    t.publish(&old).await.unwrap();
    let r = t.create("0.5.0").await;
    let a = t.upload(&r, "windows-x86_64", true).await;
    let file = t.config.storage.directory.join(&a.storage_key);
    // Same-size tampering is found by the full publication hash.
    std::fs::write(&file, vec![b'x'; CONTENT.len()]).unwrap();
    assert!(matches!(
        t.publish(&r).await,
        Err(ServiceError::PublicationBlocked(_))
    ));
    assert!(
        !t.state
            .artifacts
            .verify(&t.actor, &t.app, &r.id, &a.id)
            .await
            .unwrap()
    );
    std::fs::write(&file, CONTENT).unwrap();
    sqlx::query("CREATE TRIGGER fail_publication BEFORE INSERT ON publication_history BEGIN SELECT RAISE(ABORT,'test failure'); END").execute(&t.state.database).await.unwrap();
    assert!(matches!(
        t.publish(&r).await,
        Err(ServiceError::Internal(_))
    ));
    assert_eq!(
        t.state.releases.get(&t.app, &r.id).await.unwrap().status,
        veruna_releases::domain::release::ReleaseStatus::Draft
    );
    assert_eq!(
        t.state
            .publishing
            .history(&t.app, 10, None)
            .await
            .unwrap()
            .len(),
        1
    );
    sqlx::query("DROP TRIGGER fail_publication")
        .execute(&t.state.database)
        .await
        .unwrap();
    t.publish(&r).await.unwrap();
    // Simulate out-of-band DB corruption. Normal APIs/DB triggers forbid this.
    sqlx::query("DROP TRIGGER published_artifact_update")
        .execute(&t.state.database)
        .await
        .unwrap();
    sqlx::query("UPDATE artifacts SET standalone_signature='invalid' WHERE id=?")
        .bind(&a.id)
        .execute(&t.state.database)
        .await
        .unwrap();
    let blocked = t.update("windows-x86_64", "0.1.0").await;
    assert_eq!(blocked.status(), 503);
    assert_eq!(
        body(blocked).await["error"]["code"],
        "standalone_unavailable"
    );
    // JWS readiness never gates ordinary delivery of an intact published file.
    let manual = t
        .get(&t.state.distribution.artifact_path("product", &r, &a))
        .await;
    assert_eq!(manual.status(), 200);
    assert_eq!(
        manual
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .as_ref(),
        CONTENT
    );
    sqlx::query("UPDATE artifacts SET standalone_signature=? WHERE id=?")
        .bind(&a.standalone_signature)
        .bind(&a.id)
        .execute(&t.state.database)
        .await
        .unwrap();
    std::fs::write(&file, b"truncated").unwrap();
    assert_eq!(t.update("windows-x86_64", "0.1.0").await.status(), 503);
    std::fs::remove_file(&file).unwrap();
    assert_eq!(t.update("windows-x86_64", "0.1.0").await.status(), 503);
    assert_eq!(
        t.get(&t.state.distribution.artifact_path("product", &r, &a))
            .await
            .status(),
        503
    );
}

#[tokio::test]
async fn interrupted_upload_append_failure_and_concurrent_version_changes_reclaim_files() {
    let t = TestApp::new().await;
    let r = t.create("0.5.0").await;
    let jws = sign(&metadata(&t.app, "0.5.0", "windows-x86_64"));
    assert_eq!(
        t.multipart(&r, "windows-x86_64", Some(&jws), true)
            .await
            .status(),
        422
    );
    let mut abandoned = t.begin(&r, "windows-x86_64").await;
    abandoned.write(CONTENT).await.unwrap();
    abandoned.set_standalone_signature(jws.as_bytes()).unwrap();
    drop(abandoned);
    t.assert_storage_count(0).await;
    let mut pending = t.begin(&r, "windows-x86_64").await;
    pending.write(CONTENT).await.unwrap();
    pending.set_standalone_signature(jws.as_bytes()).unwrap();
    t.state
        .releases
        .update(
            &t.actor,
            &t.app,
            &r.id,
            ReleaseChanges {
                version: Some("0.6.0".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        t.state.artifacts.complete(&t.actor, pending).await,
        Err(ServiceError::InvalidStandalone)
    ));
    t.assert_storage_count(0).await;
    let current = t.state.releases.get(&t.app, &r.id).await.unwrap();
    t.upload(&current, "darwin-aarch64", true).await;
    t.publish(&current).await.unwrap();
    sqlx::query("CREATE TRIGGER fail_append BEFORE UPDATE OF revision ON releases BEGIN SELECT RAISE(ABORT,'test failure'); END").execute(&t.state.database).await.unwrap();
    assert_eq!(
        t.multipart(
            &current,
            "windows-x86_64",
            Some(&sign(&metadata(&t.app, "0.6.0", "windows-x86_64"))),
            false
        )
        .await
        .status(),
        500
    );
    t.assert_storage_count(1).await;
    assert_eq!(t.update("windows-x86_64", "0.1.0").await.status(), 204);
    sqlx::query("DROP TRIGGER fail_append")
        .execute(&t.state.database)
        .await
        .unwrap();
    assert_eq!(
        t.multipart(
            &current,
            "windows-x86_64",
            Some(&sign(&metadata(&t.app, "0.6.0", "windows-x86_64"))),
            false
        )
        .await
        .status(),
        201
    );
    assert_eq!(t.update("windows-x86_64", "0.1.0").await.status(), 200);
}

#[tokio::test]
async fn signed_draft_changes_stale_publish_and_concurrent_append_preserve_invariants() {
    let t = TestApp::new().await;
    let r = t.create("0.5.0").await;
    let a = t.upload(&r, "windows-x86_64", true).await;
    let stale = t.state.releases.get(&t.app, &r.id).await.unwrap().revision;
    t.state
        .releases
        .update(
            &t.actor,
            &t.app,
            &r.id,
            ReleaseChanges {
                version: Some("0.6.0".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        t.publish(&r).await,
        Err(ServiceError::PublicationBlocked(_))
    ));
    t.state
        .releases
        .update(
            &t.actor,
            &t.app,
            &r.id,
            ReleaseChanges {
                version: Some("0.5.0".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        t.state
            .publishing
            .publish(
                &t.actor,
                &t.app,
                &r.id,
                PublicationCommand {
                    expected_release_revision: stale
                }
            )
            .await,
        Err(ServiceError::PublicationConflict)
    ));
    t.publish(&r).await.unwrap();
    let mut one = t.begin(&r, "linux-x86_64").await;
    let mut two = t.begin(&r, "linux-x86_64").await;
    for upload in [&mut one, &mut two] {
        upload.write(CONTENT).await.unwrap();
        upload
            .set_standalone_signature(sign(&metadata(&t.app, "0.5.0", "linux-x86_64")).as_bytes())
            .unwrap();
    }
    let (one, two) = tokio::join!(
        t.state.artifacts.complete(&t.actor, one),
        t.state.artifacts.complete(&t.actor, two)
    );
    assert!(matches!(
        (one, two),
        (Ok(_), Err(ServiceError::ArtifactConflict)) | (Err(ServiceError::ArtifactConflict), Ok(_))
    ));
    assert_eq!(
        t.state.artifacts.list(&t.app, &r.id).await.unwrap().len(),
        2
    );
    // Database immutability still covers the new signed metadata column.
    assert!(
        sqlx::query("UPDATE artifacts SET standalone_signature=NULL WHERE id=?")
            .bind(&a.id)
            .execute(&t.state.database)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE artifacts SET distribution_role='updater' WHERE id=?")
            .bind(&a.id)
            .execute(&t.state.database)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&t.state.database)
            .await
            .unwrap()
            .is_empty()
    );
    t.assert_storage_count(2).await;
}

#[tokio::test]
async fn offline_documented_example_interoperates_with_server_and_rejects_tampering() {
    let t = TestApp::new().await;
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("docs/integrations/standalone/example.mjs");
    let prefix = t._dir.path().join("test-key");
    let output = tokio::process::Command::new("node")
        .arg(&script)
        .arg("keygen")
        .arg(&prefix)
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let key: Value = serde_json::from_slice(&output.stdout).unwrap();
    t.state
        .applications
        .set_standalone_key(&t.actor, &t.app, key["public_key"].as_str())
        .await
        .unwrap();
    let r = t.create("0.5.0").await;
    let file = t._dir.path().join("product.exe");
    std::fs::write(&file, CONTENT).unwrap();
    let output = tokio::process::Command::new("node")
        .arg(&script)
        .arg("sign")
        .arg(prefix.with_extension("pem"))
        .arg(&t.app)
        .args(["0.5.0", "windows-x86_64"])
        .arg(&file)
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let jws = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        t.multipart(&r, "windows-x86_64", Some(&jws), false)
            .await
            .status(),
        201
    );
    assert!(
        jsonwebtoken::decode_header(jws.trim())
            .unwrap()
            .kid
            .is_none()
    );
    // Explicit opt-in changes only the JWS header, never the stored executable.
    let with_kid = tokio::process::Command::new("node")
        .arg(&script)
        .arg("sign")
        .arg(prefix.with_extension("pem"))
        .arg(&t.app)
        .args(["0.5.0", "windows-x86_64"])
        .arg(&file)
        .arg("--kid")
        .output()
        .await
        .unwrap();
    assert!(
        with_kid.status.success(),
        "{}",
        String::from_utf8_lossy(&with_kid.stderr)
    );
    let with_kid = String::from_utf8(with_kid.stdout).unwrap();
    let a = t
        .state
        .artifacts
        .list(&t.app, &r.id)
        .await
        .unwrap()
        .remove(0);
    t.state
        .artifacts
        .set_standalone(&t.actor, &t.app, &r.id, &a.id, Some(with_kid.trim()))
        .await
        .unwrap();
    assert_eq!(
        jsonwebtoken::decode_header(with_kid.trim()).unwrap().kid,
        standalone::key_id(key["public_key"].as_str().unwrap())
    );
    t.publish(&r).await.unwrap();
    let update = body(t.update("windows-x86_64", "0.4.0").await).await;
    let jws_file = t._dir.path().join("product.jws");
    std::fs::write(&jws_file, update["signed_metadata"].as_str().unwrap()).unwrap();
    let url = url::Url::parse(update["url"].as_str().unwrap()).unwrap();
    let bytes = t
        .get(url.path())
        .await
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    std::fs::write(&file, bytes).unwrap();
    for tamper in [false, true] {
        if tamper {
            std::fs::write(&file, b"changed content").unwrap();
        }
        let output = tokio::process::Command::new("node")
            .arg(&script)
            .arg("verify")
            .arg(prefix.with_extension("public.jwk"))
            .arg(&t.app)
            .args(["0.5.0", "windows-x86_64"])
            .arg(&file)
            .arg(&jws_file)
            .output()
            .await
            .unwrap();
        assert_eq!(output.status.success(), !tamper);
    }
}

#[tokio::test]
async fn publication_races_with_removing_standalone_designation_never_commit_stale_snapshot() {
    let t = TestApp::new().await;
    let r = t.create("0.5.0").await;
    let a = t.upload(&r, "windows-x86_64", true).await;
    let revision = t.state.releases.get(&t.app, &r.id).await.unwrap().revision;
    let (publication, removal) = tokio::join!(
        t.state.publishing.publish(
            &t.actor,
            &t.app,
            &r.id,
            PublicationCommand {
                expected_release_revision: revision
            }
        ),
        t.state
            .artifacts
            .set_standalone(&t.actor, &t.app, &r.id, &a.id, None)
    );
    match (publication, removal) {
        (Ok(_), Err(ServiceError::ReleaseLocked)) => {
            assert_eq!(t.update("windows-x86_64", "0.1.0").await.status(), 200);
        }
        (Err(ServiceError::PublicationConflict), Ok(_)) => {
            assert_eq!(
                t.state.releases.get(&t.app, &r.id).await.unwrap().status,
                veruna_releases::domain::release::ReleaseStatus::Draft
            );
            assert_eq!(t.update("windows-x86_64", "0.1.0").await.status(), 204);
        }
        other => panic!("Unexpected publication/removal result: {other:?}"),
    }
}

#[tokio::test]
async fn optional_key_id_is_bound_to_trusted_key_and_legacy_signatures_still_work() {
    let t = TestApp::new().await;
    let r = t.create("0.5.0").await;
    let a = t.upload(&r, "windows-x86_64", false).await;
    // RFC 8037 A.3: the RFC 7638 SHA-256 thumbprint of this test public JWK.
    let kid = standalone::key_id(PUBLIC_KEY).unwrap();
    assert_eq!(kid, "kPrK_qmxVWaYVA9wwBF6Iuo3vVzz7TxHCTwXBygrS4k");
    let settings = body(
        t.admin(
            "GET",
            &format!("{}/integrations/standalone", t.base()),
            Value::Null,
            Some(&t.token),
            true,
        )
        .await,
    )
    .await;
    assert_eq!(settings["key_id"], kid);
    let payload = metadata(&t.app, "0.5.0", "windows-x86_64");
    let mut header = Header::new(Algorithm::EdDSA);
    header.typ = Some(standalone::MEDIA_TYPE.into());
    for invalid in ["", "unknown-key", "https://untrusted.invalid/key.jwk"] {
        header.kid = Some(invalid.into());
        let signature = sign_header(&payload, &header, &SEED);
        assert!(matches!(
            t.state
                .artifacts
                .set_standalone(&t.actor, &t.app, &r.id, &a.id, Some(&signature))
                .await,
            Err(ServiceError::InvalidStandalone)
        ));
    }
    header.kid = None;
    header.extras.insert("kid", Value::Null);
    let signature = sign_header(&payload, &header, &SEED);
    assert!(matches!(
        t.state
            .artifacts
            .set_standalone(&t.actor, &t.app, &r.id, &a.id, Some(&signature))
            .await,
        Err(ServiceError::InvalidStandalone)
    ));
    // No identifier remains a valid, unambiguous reference to the one pinned key.
    for kid in [None, Some(kid)] {
        let mut header = Header::new(Algorithm::EdDSA);
        header.typ = Some(standalone::MEDIA_TYPE.into());
        header.kid = kid;
        let signature = sign_header(&payload, &header, &SEED);
        t.state
            .artifacts
            .set_standalone(&t.actor, &t.app, &r.id, &a.id, Some(&signature))
            .await
            .unwrap();
    }
    t.publish(&r).await.unwrap();
    let update = body(t.update("windows-x86_64", "0.4.0").await).await;
    let header = jsonwebtoken::decode_header(update["signed_metadata"].as_str().unwrap()).unwrap();
    assert_eq!(header.kid, standalone::key_id(PUBLIC_KEY));
    assert_eq!(
        t.state.artifacts.list(&t.app, &r.id).await.unwrap().len(),
        1
    );
    t.assert_storage_count(1).await;
}
