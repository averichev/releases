use axum::{Router, body::Body, http::Request, response::Response};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tempfile::{TempDir, tempdir};
use tower::ServiceExt;
use veruna_releases::{
    application_state,
    config::AppConfig,
    domain::{
        application::{NewApplication, Status, Visibility},
        artifact::{Artifact, UploadMetadata},
        release::{Release, ReleaseChanges, ReleaseInput, ReleaseStatus},
        user::Role,
    },
    http::{self, AppState},
    infrastructure::secrets,
    initialize,
    services::{
        error::ServiceError,
        publishing::PublicationCommand,
        sessions::Identity,
        updater::{self, UpdateDecision},
    },
};
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
    fn release_path(&self, r: &Release) -> String {
        format!("{}/releases/{}", self.base(), r.id)
    }
    fn artifact_path(&self, r: &Release, a: &Artifact) -> String {
        format!("{}/artifacts/{}", self.release_path(r), a.id)
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
                    release_notes: "Исправления\nи улучшения".into(),
                },
            )
            .await
            .unwrap()
    }
    async fn upload(&self, r: &Release, target: &str, filename: &str) -> Artifact {
        let mut upload = self
            .state
            .artifacts
            .begin(
                &self.actor,
                &self.app,
                &r.id,
                UploadMetadata {
                    filename: filename.into(),
                    target: target.into(),
                    file_type: None,
                    distribution_role: None,
                },
            )
            .await
            .unwrap();
        upload.write(b"package").await.unwrap();
        self.state
            .artifacts
            .complete(&self.actor, upload)
            .await
            .unwrap()
    }
    async fn command(&self, r: &Release) -> PublicationCommand {
        PublicationCommand {
            expected_release_revision: self
                .state
                .releases
                .get(&self.app, &r.id)
                .await
                .unwrap()
                .revision,
        }
    }
    async fn publish(&self, r: &Release) {
        self.state
            .publishing
            .publish(&self.actor, &self.app, &r.id, self.command(r).await)
            .await
            .unwrap();
    }
    async fn request(
        &self,
        method: &str,
        path: &str,
        body: Body,
        token: Option<&str>,
        csrf: bool,
    ) -> Response {
        let mut request = Request::builder()
            .uri(path)
            .method(method)
            .header("origin", &self.config.auth.public_origin)
            .header("x-veruna-request", "1")
            .header("content-type", "application/json");
        if let Some(token) = token {
            request = request.header("cookie", format!("veruna_session={token}"));
            if csrf {
                request = request.header("x-csrf-token", secrets::csrf(token));
            }
        }
        self.router
            .clone()
            .oneshot(request.body(body).unwrap())
            .await
            .unwrap()
    }
    async fn call(&self, method: &str, path: &str, body: Value, status: u16) -> Value {
        let response = self
            .request(
                method,
                path,
                Body::from(body.to_string()),
                Some(&self.token),
                true,
            )
            .await;
        let actual = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(
            actual,
            status,
            "{method} {path}: {}",
            String::from_utf8_lossy(&bytes)
        );
        if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        }
    }
    async fn corrupt(&self, a: &Artifact, bytes: &[u8]) {
        tokio::fs::write(self.config.storage.directory.join(&a.storage_key), bytes)
            .await
            .unwrap();
    }
}
fn signature() -> String {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let mut packet = vec![0u8; 74];
    packet[..2].copy_from_slice(b"ED");
    STANDARD.encode(format!("untrusted comment: signature from minisign secret key\n{}\ntrusted comment: timestamp:1\n{}\n",STANDARD.encode(packet),STANDARD.encode([0u8;64])))
}
fn command_json(c: PublicationCommand) -> Value {
    json!({"expected_release_revision":c.expected_release_revision})
}
#[tokio::test]
async fn publication_checks_lifecycle_http_and_sql_immutability_and_idempotency() {
    let t = TestApp::new().await;
    let r = t.create("1.4.0+ci.5").await;
    let a = t.upload(&r, "windows-x86_64", "setup.exe").await;
    let check = t
        .call(
            "POST",
            &format!("{}/validate-publication", t.release_path(&r)),
            json!({}),
            200,
        )
        .await;
    assert_eq!(check["validation"]["state"], "ready");
    let command = command_json(t.command(&r).await);
    let path = format!("{}/publish", t.release_path(&r));
    let first = t.call("POST", &path, command.clone(), 200).await;
    let published = t.state.releases.get(&t.app, &r.id).await.unwrap();
    assert_eq!(published.status, ReleaseStatus::Published);
    assert!(published.published_at.is_some());

    assert_eq!(t.call("POST", &path, command, 200).await, first);
    assert_eq!(
        t.state
            .releases
            .get(&t.app, &r.id)
            .await
            .unwrap()
            .published_at,
        published.published_at
    );
    assert_eq!(
        t.state
            .publishing
            .history(&t.app, 50, None)
            .await
            .unwrap()
            .len(),
        1
    );
    t.call(
        "PATCH",
        &t.release_path(&r),
        json!({"version":"9.0.0"}),
        409,
    )
    .await;
    t.call("DELETE", &t.release_path(&r), Value::Null, 409)
        .await;
    t.call("DELETE", &t.artifact_path(&r, &a), Value::Null, 409)
        .await;
    t.call(
        "PUT",
        &format!("{}/updater", t.artifact_path(&r, &a)),
        json!({"distribution_role":"updater"}),
        409,
    )
    .await;
    assert!(
        t.state
            .artifacts
            .begin(
                &t.actor,
                &t.app,
                &r.id,
                UploadMetadata {
                    filename: "new.zip".into(),
                    target: "any".into(),
                    file_type: None,
                    distribution_role: None,
                }
            )
            .await
            .is_ok()
    );
    for sql in [
        "UPDATE releases SET release_notes='changed' WHERE status='published'",
        "UPDATE releases SET status='draft',published_at=NULL WHERE status='published'",
        "UPDATE releases SET published_at=published_at+1 WHERE status='published'",
        "DELETE FROM releases WHERE status='published'",
        "DELETE FROM artifacts",
        "UPDATE artifacts SET sha256=replace(sha256,'a','b')",
        "UPDATE artifacts SET updater_signature=NULL",
        "DELETE FROM publication_history",
        "UPDATE publication_history SET created_at=created_at+1",
    ] {
        assert!(
            sqlx::query(sql).execute(&t.state.database).await.is_err(),
            "{sql}"
        );
    }
    let check = t
        .call(
            "POST",
            &format!("{}/validate-publication", t.release_path(&r)),
            json!({}),
            200,
        )
        .await;
    assert_eq!(
        check["validation"]["errors"][0]["code"],
        "release_published"
    );
}
#[tokio::test]
async fn validation_explains_missing_corrupt_and_archived_releases_without_state_changes() {
    let t = TestApp::new().await;
    let r = t.create("1.0.0").await;
    let validation = t
        .state
        .publishing
        .validate(&t.actor, &t.app, &r.id)
        .await
        .unwrap();
    assert_eq!(validation.validation.errors[0].code, "no_artifacts");
    t.call(
        "POST",
        &format!("{}/publish", t.release_path(&r)),
        command_json(t.command(&r).await),
        422,
    )
    .await;
    let a = t.upload(&r, "linux-x86_64", "package.deb").await;
    t.corrupt(&a, b"damaged").await;
    let validation = t
        .state
        .publishing
        .validate(&t.actor, &t.app, &r.id)
        .await
        .unwrap();
    assert_eq!(validation.validation.errors[0].code, "checksum_mismatch");
    let result = t
        .call(
            "POST",
            &format!("{}/publish", t.release_path(&r)),
            command_json(t.command(&r).await),
            422,
        )
        .await;
    assert_eq!(result["validation"]["errors"][0]["artifact_id"], a.id);
    t.corrupt(&a, b"x").await;
    let validation = t
        .state
        .publishing
        .validate(&t.actor, &t.app, &r.id)
        .await
        .unwrap();
    assert_eq!(validation.validation.errors[0].code, "file_unavailable");
    tokio::fs::remove_file(t.config.storage.directory.join(&a.storage_key))
        .await
        .unwrap();
    assert_eq!(
        t.state
            .publishing
            .validate(&t.actor, &t.app, &r.id)
            .await
            .unwrap()
            .validation
            .errors[0]
            .code,
        "file_unavailable"
    );
    t.corrupt(&a, b"package").await;
    t.state
        .applications
        .set_status(&t.actor, &t.app, Status::Archived)
        .await
        .unwrap();
    assert_eq!(
        t.state
            .publishing
            .validate(&t.actor, &t.app, &r.id)
            .await
            .unwrap()
            .validation
            .errors[0]
            .code,
        "application_archived"
    );
    assert!(matches!(
        t.state
            .publishing
            .publish(&t.actor, &t.app, &r.id, t.command(&r).await)
            .await,
        Err(ServiceError::PublicationBlocked(_))
    ));
    assert_eq!(
        t.state.releases.get(&t.app, &r.id).await.unwrap().status,
        ReleaseStatus::Draft
    );
    assert!(
        t.state
            .publishing
            .history(&t.app, 50, None)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn publication_ownership_and_optimistic_revisions_reject_stale_and_parallel_commands() {
    let t = TestApp::new().await;
    let a = t.create("1.0.0").await;
    t.upload(&a, "any", "one.zip").await;
    let b = t.create("2.0.0").await;
    t.upload(&b, "any", "two.zip").await;
    t.state
        .users
        .create(
            &t.actor,
            "second-admin",
            Role::Admin,
            Some("test-password-123".into()),
        )
        .await
        .unwrap();
    let login = t
        .state
        .authentication
        .login("second-admin", "test-password-123".into(), None)
        .await
        .unwrap();
    let second = t.state.sessions.authenticate(&login.token).await.unwrap();
    let ca = t.command(&a).await;
    let cb = t.command(&b).await;
    let (one, two) = tokio::join!(
        t.state.publishing.publish(&t.actor, &t.app, &a.id, ca),
        t.state.publishing.publish(&second, &t.app, &b.id, cb)
    );
    assert!(one.is_ok() && two.is_ok());
    assert_eq!(
        t.state
            .publishing
            .history(&t.app, 50, None)
            .await
            .unwrap()
            .len(),
        2
    );
    let draft = t.create("3.0.0").await;
    t.upload(&draft, "any", "three.zip").await;
    let stale = t.command(&draft).await;
    t.state
        .releases
        .update(
            &t.actor,
            &t.app,
            &draft.id,
            ReleaseChanges {
                release_notes: Some("Concurrent edit".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        t.state
            .publishing
            .publish(&t.actor, &t.app, &draft.id, stale)
            .await,
        Err(ServiceError::PublicationConflict)
    ));
    let foreign = t
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
            .publishing
            .publish(
                &t.actor,
                &foreign.id,
                &a.id,
                PublicationCommand {
                    expected_release_revision: 1
                }
            )
            .await,
        Err(ServiceError::NotFound)
    ));
}

#[tokio::test]
async fn updater_roles_signature_lifecycle_and_public_diagnostics() {
    let t = TestApp::new().await;
    let r = t.create("1.2.0").await;
    let a = t.upload(&r, "windows-x86_64", "setup.exe").await;
    let path = t.artifact_path(&r, &a);
    let sig = signature();
    assert_eq!(updater::signature(sig.as_bytes()).unwrap(), sig);
    let url = format!("{path}/signature");
    assert_eq!(
        t.request("PUT", &url, Body::from(sig.clone()), Some(&t.token), true)
            .await
            .status(),
        422
    );
    t.call(
        "PUT",
        &format!("{path}/updater"),
        json!({"distribution_role":"updater"}),
        200,
    )
    .await;
    let check = t
        .state
        .publishing
        .validate(&t.actor, &t.app, &r.id)
        .await
        .unwrap();
    assert!(check.validation.errors.is_empty());
    assert_eq!(check.validation.warnings[0].code, "signature_missing");
    for invalid in [
        "private key",
        "file.sig",
        "https://example.test/file.sig",
        "a".repeat(64).as_str(),
    ] {
        assert_eq!(
            t.request(
                "PUT",
                &url,
                Body::from(invalid.to_owned()),
                Some(&t.token),
                true
            )
            .await
            .status(),
            422
        );
    }
    assert_eq!(
        t.request(
            "PUT",
            &url,
            Body::from(vec![b'x'; 16385]),
            Some(&t.token),
            true
        )
        .await
        .status(),
        413
    );
    assert_eq!(
        t.request("PUT", &url, Body::from(sig.clone()), Some(&t.token), true)
            .await
            .status(),
        200
    );
    t.call("DELETE", &url, Value::Null, 200).await;
    assert_eq!(
        t.request("PUT", &url, Body::from(sig.clone()), Some(&t.token), true)
            .await
            .status(),
        200
    );
    t.publish(&r).await;
    assert_eq!(
        t.request("PUT", &url, Body::from(sig), Some(&t.token), true)
            .await
            .status(),
        409
    );
    t.call("DELETE", &url, Value::Null, 409).await;
    let preview_path = format!("{}/updater/tauri/preview", t.base());
    let preview = t
        .call(
            "POST",
            &preview_path,
            json!({"target":"windows-x86_64","current_version":"1.0.0"}),
            200,
        )
        .await;
    assert_eq!(preview["reason"], "update_available");
    assert_eq!(preview["artifact_id"], a.id);
    assert!(preview["response"]["url"].as_str().unwrap().contains(&a.id));
    assert_eq!(preview["public_downloads_available"], true);
    for version in ["1.2.0", "1.2.0+different", "9.0.0"] {
        assert_eq!(
            t.call(
                "POST",
                &preview_path,
                json!({"target":"windows-x86_64","current_version":version}),
                200
            )
            .await["reason"],
            "version_not_newer"
        );
    }
    assert_eq!(
        t.call(
            "POST",
            &preview_path,
            json!({"target":"darwin-aarch64","current_version":"1.0.0"}),
            200
        )
        .await["reason"],
        "updater_package_missing"
    );
    t.call(
        "POST",
        &preview_path,
        json!({"target":"windows-x86_64","current_version":"garbage"}),
        422,
    )
    .await;
    t.call("GET", "/product/updater/tauri", Value::Null, 404)
        .await;
    t.corrupt(&a, b"truncated").await;
    assert_eq!(
        t.call(
            "POST",
            &preview_path,
            json!({"target":"windows-x86_64","current_version":"1.0.0"}),
            200
        )
        .await["reason"],
        "artifact_corrupted"
    );
}
#[tokio::test]
async fn tauri_protocol_compares_semver_maps_targets_and_requires_explicit_signed_compatible_packages()
 {
    let t = TestApp::new().await;
    let r = t.create("1.2.0+ci.1").await;
    let mut a = t.upload(&r, "windows-x86_64", "setup.exe").await;
    t.publish(&r).await;
    let r = t.state.releases.get(&t.app, &r.id).await.unwrap();
    assert!(matches!(
        updater::select(r.clone(), vec![a.clone()], "windows-x64", "1.0.0").unwrap(),
        UpdateDecision::MissingPackage
    ));
    a.distribution_role = "updater".into();
    assert!(matches!(
        updater::select(r.clone(), vec![a.clone()], "windows-x64", "1.0.0").unwrap(),
        UpdateDecision::MissingSignature
    ));
    a.updater_signature = Some(signature());
    let UpdateDecision::Available(selection) =
        updater::select(r.clone(), vec![a.clone()], "windows-x64", "1.1.9").unwrap()
    else {
        panic!("update expected")
    };
    let body = updater::response(
        &selection,
        "https://apps.example.test",
        &format!("/downloads/{}", a.id),
    )
    .unwrap();
    let value = serde_json::to_value(body).unwrap();
    assert_eq!(value["version"], "1.2.0+ci.1");
    assert_eq!(value["signature"], signature());
    assert_eq!(
        value["url"],
        format!("https://apps.example.test/downloads/{}", a.id)
    );
    assert!(value["pub_date"].as_str().unwrap().ends_with('Z'));
    for url in [
        "//evil.test/file",
        "https://evil.test/file",
        "/downloads/wrong",
        "/downloads/../wrong",
    ] {
        assert!(updater::response(&selection, "https://apps.example.test", url).is_err());
    }
    for current in ["1.2.0", "1.2.0+ci.2", "2.0.0"] {
        assert!(matches!(
            updater::select(r.clone(), vec![a.clone()], "windows-x86_64", current).unwrap(),
            UpdateDecision::NoUpdate
        ));
    }
    assert!(updater::select(r.clone(), vec![a.clone()], "windows-x86_64", "1.0").is_err());
    assert!(matches!(
        updater::select(
            r.clone(),
            vec![a.clone(), a.clone()],
            "windows-x86_64",
            "1.0.0"
        )
        .unwrap(),
        UpdateDecision::AmbiguousPackage
    ));
    for (internal, target, format) in [
        ("windows-aarch64", "windows-arm64", "msi"),
        ("darwin-aarch64", "macos-arm64", "tar.gz"),
        ("darwin-universal", "macos-universal", "tar.gz"),
        ("linux-x86_64", "linux-x64", "appimage"),
    ] {
        a.target = internal.into();
        a.file_type = format.into();
        assert!(matches!(
            updater::select(r.clone(), vec![a.clone()], target, "1.0.0").unwrap(),
            UpdateDecision::Available(_)
        ));
    }
    for (target, format) in [
        ("windows-x86_64", "zip"),
        ("darwin-aarch64", "dmg"),
        ("darwin-universal", "dmg"),
        ("linux-x86_64", "deb"),
    ] {
        a.target = target.into();
        a.file_type = format.into();
        assert!(matches!(
            updater::select(r.clone(), vec![a.clone()], target, "1.0.0").unwrap(),
            UpdateDecision::MissingPackage
        ));
    }
    a.target = "darwin-universal".into();
    a.file_type = "tar.gz".into();
    for target in ["darwin-x86_64", "macos-arm64"] {
        assert!(matches!(
            updater::select(r.clone(), vec![a.clone()], target, "1.0.0").unwrap(),
            UpdateDecision::Available(_)
        ));
    }
    for target in [
        "windows-x86_64",
        "linux-aarch64",
        "darwin-i686",
        "darwin-armv7",
    ] {
        assert!(matches!(
            updater::select(r.clone(), vec![a.clone()], target, "1.0.0").unwrap(),
            UpdateDecision::MissingPackage
        ));
    }
    for target in ["windows-universal", "linux-universal"] {
        assert!(updater::select(r.clone(), vec![a.clone()], target, "1.0.0").is_err());
    }
    for target in ["darwin-x86_64", "darwin-aarch64"] {
        let mut exact = a.clone();
        exact.id = "exact-package".into();
        exact.target = target.into();
        for artifacts in [
            vec![a.clone(), exact.clone()],
            vec![exact.clone(), a.clone()],
        ] {
            let UpdateDecision::Available(selection) =
                updater::select(r.clone(), artifacts, target, "1.0.0").unwrap()
            else {
                panic!("exact package expected");
            };
            assert_eq!(selection.artifact.id, exact.id);
        }
        assert!(matches!(
            updater::select(
                r.clone(),
                vec![a.clone(), exact.clone(), exact.clone()],
                target,
                "1.0.0"
            )
            .unwrap(),
            UpdateDecision::AmbiguousPackage
        ));
        exact.updater_signature = None;
        assert!(matches!(
            updater::select(r.clone(), vec![a.clone(), exact], target, "1.0.0").unwrap(),
            UpdateDecision::MissingSignature
        ));
    }
    a.updater_signature = None;
    assert!(matches!(
        updater::select(r.clone(), vec![a.clone()], "darwin-aarch64", "1.0.0").unwrap(),
        UpdateDecision::MissingSignature
    ));
    a.updater_signature = Some(signature());
    assert!(matches!(
        updater::select(
            r.clone(),
            vec![a.clone(), a.clone()],
            "darwin-aarch64",
            "1.0.0"
        )
        .unwrap(),
        UpdateDecision::AmbiguousPackage
    ));
}
#[tokio::test]
async fn new_admin_endpoints_enforce_sessions_roles_csrf_and_revocation() {
    let t = TestApp::new().await;
    let r = t.create("1.0.0").await;
    let a = t.upload(&r, "windows-x86_64", "setup.exe").await;
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
    for (method, path, readable) in [
        ("GET", format!("{}/publication-history", t.base()), true),
        (
            "POST",
            format!("{}/validate-publication", t.release_path(&r)),
            false,
        ),
        ("POST", format!("{}/publish", t.release_path(&r)), false),
        ("GET", format!("{}/manifests/preview", t.base()), false),
        ("POST", format!("{}/updater/tauri/preview", t.base()), true),
        ("PUT", format!("{}/updater", t.artifact_path(&r, &a)), false),
        (
            "PUT",
            format!("{}/signature", t.artifact_path(&r, &a)),
            false,
        ),
        (
            "DELETE",
            format!("{}/signature", t.artifact_path(&r, &a)),
            false,
        ),
    ] {
        assert_eq!(
            t.request(method, &path, Body::from("{}"), None, true)
                .await
                .status(),
            401,
            "{method} {path}"
        );
        assert_eq!(
            t.request(
                method,
                &path,
                Body::from(if path.ends_with("/updater/tauri/preview") {
                    r#"{"target":"windows-x86_64","current_version":"1.0.0"}"#
                } else {
                    "{}"
                }),
                Some(&operator),
                true
            )
            .await
            .status(),
            if readable { 200 } else { 403 },
            "{method} {path}"
        );
        if method != "GET" {
            assert_eq!(
                t.request(method, &path, Body::from("{}"), Some(&t.token), false)
                    .await
                    .status(),
                403
            );
        }
    }
    t.state.sessions.logout(&t.actor).await.unwrap();
    assert!(matches!(
        t.state
            .publishing
            .publish(
                &t.actor,
                &t.app,
                &r.id,
                PublicationCommand {
                    expected_release_revision: 1
                }
            )
            .await,
        Err(ServiceError::Unauthorized)
    ));
}

#[tokio::test]
async fn updater_conflict_and_inflight_installer_recheck_published_selection() {
    let t = TestApp::new().await;
    let r = t.create("1.0.0").await;
    let one = t.upload(&r, "windows-x86_64", "setup.exe").await;
    let two = t.upload(&r, "windows-x86_64", "setup.msi").await;
    t.state
        .artifacts
        .set_updater(&t.actor, &t.app, &r.id, &one.id, Some("updater"), None)
        .await
        .unwrap();
    assert!(matches!(
        t.state
            .artifacts
            .set_updater(&t.actor, &t.app, &r.id, &two.id, Some("updater"), None)
            .await,
        Err(ServiceError::ArtifactConflict)
    ));
    let mut pending = t
        .state
        .artifacts
        .begin(
            &t.actor,
            &t.app,
            &r.id,
            UploadMetadata {
                filename: "linux.deb".into(),
                target: "linux-x86_64".into(),
                file_type: None,
                distribution_role: None,
            },
        )
        .await
        .unwrap();
    pending.write(b"pending").await.unwrap();
    t.publish(&r).await;
    // Publication while bytes are in flight must activate published append rules.
    t.state.artifacts.complete(&t.actor, pending).await.unwrap();
    assert_eq!(
        t.state.artifacts.list(&t.app, &r.id).await.unwrap().len(),
        3
    );
    assert_eq!(
        t.state
            .updater
            .preview(&t.app, "windows-x86_64", "0.0.0")
            .await
            .unwrap()
            .reason,
        "signature_missing"
    );
}

#[tokio::test]
async fn history_failure_rolls_back_publication() {
    let t = TestApp::new().await;
    let r = t.create("1.0.0").await;
    t.upload(&r, "any", "package.zip").await;
    sqlx::query("CREATE TRIGGER fail_publication_history BEFORE INSERT ON publication_history BEGIN SELECT RAISE(ABORT,'test failure'); END").execute(&t.state.database).await.unwrap();
    assert!(
        t.state
            .publishing
            .publish(&t.actor, &t.app, &r.id, t.command(&r).await)
            .await
            .is_err()
    );
    assert_eq!(
        t.state.releases.get(&t.app, &r.id).await.unwrap().status,
        ReleaseStatus::Draft
    );
    assert!(
        t.state
            .publishing
            .history(&t.app, 50, None)
            .await
            .unwrap()
            .is_empty()
    );
    sqlx::query("DROP TRIGGER fail_publication_history")
        .execute(&t.state.database)
        .await
        .unwrap();
    t.publish(&r).await;
}

impl TestApp {
    async fn upload_form(
        &self,
        r: &Release,
        target: &str,
        filename: &str,
        role: Option<&str>,
        sig: Option<&str>,
    ) -> Response {
        let mut body = format!(
            "--UPLOAD\r\nContent-Disposition: form-data; name=\"target\"\r\n\r\n{target}\r\n"
        );
        if let Some(role) = role {
            body.push_str(&format!("--UPLOAD\r\nContent-Disposition: form-data; name=\"distribution_role\"\r\n\r\n{role}\r\n"));
        }
        body.push_str(&format!("--UPLOAD\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\nContent-Type: application/octet-stream\r\n\r\npackage\r\n"));
        if let Some(sig) = sig {
            body.push_str(&format!("--UPLOAD\r\nContent-Disposition: form-data; name=\"signature\"; filename=\"{filename}.sig\"\r\n\r\n{sig}\r\n"));
        }
        body.push_str("--UPLOAD--\r\n");
        self.router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("{}/artifacts", self.release_path(r)))
                    .header("origin", &self.config.auth.public_origin)
                    .header("x-veruna-request", "1")
                    .header("x-csrf-token", secrets::csrf(&self.token))
                    .header("cookie", format!("veruna_session={}", self.token))
                    .header("content-type", "multipart/form-data; boundary=UPLOAD")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap()
    }
}
async fn response_json(response: Response, status: u16) -> Value {
    let actual = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(actual, status, "{}", String::from_utf8_lossy(&bytes));
    serde_json::from_slice(&bytes).unwrap()
}
#[tokio::test]
async fn draft_upload_roles_conflicts_and_signature_are_atomic() {
    let t = TestApp::new().await;
    let r = t.create("0.5.0").await;
    let installer = response_json(
        t.upload_form(&r, "windows-x86_64", "installer.exe", None, None)
            .await,
        201,
    )
    .await;
    assert_eq!(installer["distribution_role"], "installer"); // old clients
    let updater = response_json(
        t.upload_form(&r, "windows-x86_64", "updater.exe", Some("updater"), None)
            .await,
        201,
    )
    .await;
    assert_eq!(updater["distribution_role"], "updater");
    assert_eq!(updater["signature_present"], false);
    let before = t.state.releases.get(&t.app, &r.id).await.unwrap();
    for (filename, role) in [
        ("again.exe", "installer"),
        ("again.exe", "updater"),
        ("again.msi", "updater"),
    ] {
        assert_eq!(
            response_json(
                t.upload_form(&r, "windows-x86_64", filename, Some(role), None)
                    .await,
                409
            )
            .await["error"]["code"],
            "artifact_exists"
        );
    }
    for (id, role) in [
        (installer["id"].as_str().unwrap(), "updater"),
        (updater["id"].as_str().unwrap(), "installer"),
    ] {
        t.call(
            "PUT",
            &format!("{}/artifacts/{id}/updater", t.release_path(&r)),
            json!({"distribution_role":role}),
            409,
        )
        .await;
    }
    assert_eq!(t.state.releases.get(&t.app, &r.id).await.unwrap(), before);
    let check = t
        .state
        .publishing
        .validate(&t.actor, &t.app, &r.id)
        .await
        .unwrap();
    assert!(check.validation.errors.is_empty());
    let sig = signature();
    assert_eq!(
        t.request(
            "PUT",
            &format!(
                "{}/artifacts/{}/signature",
                t.release_path(&r),
                updater["id"].as_str().unwrap()
            ),
            Body::from(sig.clone()),
            Some(&t.token),
            true
        )
        .await
        .status(),
        200
    );
    let darwin = response_json(
        t.upload_form(
            &r,
            "darwin-aarch64",
            "update.tar.gz",
            Some("updater"),
            Some(&sig),
        )
        .await,
        201,
    )
    .await;
    assert_eq!(darwin["signature_present"], true);
    t.publish(&r).await;
    t.call(
        "POST",
        &format!("{}/releases", t.base()),
        json!({"version":"0.5.0","release_notes":""}),
        409,
    )
    .await;
}
#[tokio::test]
async fn published_append_updates_manifest_etag_downloads_and_tauri_without_changing_publication() {
    let t = TestApp::new().await;
    let r = t.create("0.5.0").await;
    let installer = t.upload(&r, "windows-x86_64", "installer.exe").await;
    t.publish(&r).await;
    let before = t.state.releases.get(&t.app, &r.id).await.unwrap();
    let history = t
        .call(
            "GET",
            &format!("{}/publication-history", t.base()),
            Value::Null,
            200,
        )
        .await;
    let old_etag = t
        .request("GET", "/product/manifest.json", Body::empty(), None, false)
        .await
        .headers()["etag"]
        .clone();
    for sig in [None, Some("invalid signature")] {
        let error = response_json(
            t.upload_form(&r, "windows-x86_64", "updater.exe", Some("updater"), sig)
                .await,
            422,
        )
        .await;
        assert_eq!(
            error["error"]["code"],
            if sig.is_none() {
                "updater_signature_required"
            } else {
                "invalid_updater"
            }
        );
        assert_eq!(t.state.releases.get(&t.app, &r.id).await.unwrap(), before);
        assert_eq!(
            t.request("GET", "/product/manifest.json", Body::empty(), None, false)
                .await
                .headers()["etag"],
            old_etag
        );
    }
    let sig = signature();
    let added = response_json(
        t.upload_form(
            &r,
            "windows-x86_64",
            "updater.exe",
            Some("updater"),
            Some(&sig),
        )
        .await,
        201,
    )
    .await;
    let updater_id = added["id"].as_str().unwrap();
    let after_updater = t.state.releases.get(&t.app, &r.id).await.unwrap();
    assert_eq!(after_updater.revision, before.revision + 1);
    assert_eq!(after_updater.published_at, before.published_at);
    assert_eq!(after_updater.updated_at, before.updated_at);
    let linux = response_json(
        t.upload_form(&r, "linux-x86_64", "installer.deb", Some("installer"), None)
            .await,
        201,
    )
    .await;
    let after = t.state.releases.get(&t.app, &r.id).await.unwrap();
    assert_eq!(after.version, before.version);
    assert_eq!(after.published_at, before.published_at);
    assert_eq!(after.updated_at, before.updated_at);
    assert_eq!(after.revision, before.revision + 2);

    assert_eq!(
        t.call(
            "GET",
            &format!("{}/publication-history", t.base()),
            Value::Null,
            200
        )
        .await,
        history
    );
    let response = t
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/product/manifest.json")
                .header("if-none-match", &old_etag)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_ne!(response.headers()["etag"], old_etag);
    let manifest = response_json(response, 200).await;
    let files = manifest["releases"][0]["artifacts"].as_array().unwrap();
    assert_eq!(files.len(), 3);
    for (id, purpose) in [
        (installer.id.as_str(), "installer"),
        (updater_id, "updater"),
        (linux["id"].as_str().unwrap(), "installer"),
    ] {
        let file = files.iter().find(|a| a["id"] == id).unwrap();
        assert_eq!(file["purpose"], purpose);
        let url = file["download_url"]
            .as_str()
            .unwrap()
            .strip_prefix(&t.config.download.public_base_url)
            .unwrap();
        for method in ["GET", "HEAD"] {
            let response = t.request(method, url, Body::empty(), None, false).await;
            assert_eq!(response.status(), 200);
            assert_eq!(response.headers()["content-length"], "7");
            let bytes = response.into_body().collect().await.unwrap().to_bytes();
            assert_eq!(
                bytes.as_ref(),
                if method == "GET" {
                    b"package".as_slice()
                } else {
                    b"".as_slice()
                }
            );
        }
    }
    for (path, id) in [
        ("/product/latest/windows-x86_64", installer.id.as_str()),
        (
            "/product/latest/windows-x86_64?format=exe",
            installer.id.as_str(),
        ),
        (
            "/product/0.5.0/windows-x86_64?purpose=updater&format=exe",
            updater_id,
        ),
        (
            "/product/latest/linux-x86_64",
            linux["id"].as_str().unwrap(),
        ),
    ] {
        let response = t.request("GET", path, Body::empty(), None, false).await;
        assert_eq!(response.status(), 307);
        assert!(
            response.headers()["location"]
                .to_str()
                .unwrap()
                .contains(id)
        );
        assert_eq!(response.headers()["cache-control"], "no-store");
    }
    let update = t
        .call(
            "GET",
            "/api/v1/updates/tauri/product/windows/x86_64/0.4.0",
            Value::Null,
            200,
        )
        .await;
    assert!(update["url"].as_str().unwrap().contains(updater_id));
    assert_eq!(update["signature"], sig);
    assert_eq!(update["version"], "0.5.0");
}
#[tokio::test]
async fn published_artifacts_cannot_be_replaced_even_with_sql_replace() {
    let t = TestApp::new().await;
    let r = t.create("0.5.0").await;
    let a = t.upload(&r, "windows-x86_64", "installer.exe").await;
    t.publish(&r).await;
    let before = t.state.artifacts.list(&t.app, &r.id).await.unwrap();
    for column in [
        "filename='renamed.exe'",
        "target='any'",
        "file_type='zip'",
        "size_bytes=size_bytes+1",
        "sha256=lower(hex(randomblob(32)))",
        "storage_key=lower(hex(randomblob(16)))||'.blob'",
        "distribution_role='updater'",
        "updater_signature=NULL",
        "id=lower(hex(randomblob(18)))",
    ] {
        assert!(
            sqlx::query(&format!("UPDATE artifacts SET {column} WHERE id=?"))
                .bind(&a.id)
                .execute(&t.state.database)
                .await
                .is_err(),
            "{column}"
        );
    }
    // Same ID, storage key, installer tuple or updater target must never remove a published row.
    for (id, key) in [
        (
            a.id.clone(),
            format!("{}.blob", uuid::Uuid::new_v4().simple()),
        ),
        (uuid::Uuid::new_v4().to_string(), a.storage_key.clone()),
        (
            uuid::Uuid::new_v4().to_string(),
            format!("{}.blob", uuid::Uuid::new_v4().simple()),
        ),
    ] {
        assert!(sqlx::query("INSERT OR REPLACE INTO artifacts(id,release_id,filename,target,file_type,size_bytes,sha256,storage_key,created_at) VALUES(?,?,'replacement.exe','windows-x86_64','exe',7,?,?,1)").bind(id).bind(&r.id).bind(&a.sha256).bind(key).execute(&t.state.database).await.is_err());
    }
    t.call("DELETE", &t.artifact_path(&r, &a), Value::Null, 409)
        .await;
    t.call(
        "PUT",
        &format!("{}/updater", t.artifact_path(&r, &a)),
        json!({"distribution_role":"updater"}),
        409,
    )
    .await;
    for (method, suffix) in [("PUT", ""), ("POST", ""), ("PUT", "/download")] {
        t.call(
            method,
            &format!("{}{suffix}", t.artifact_path(&r, &a)),
            Value::Null,
            405,
        )
        .await;
    }
    assert_eq!(t.state.artifacts.list(&t.app, &r.id).await.unwrap(), before);
    for sql in [
        "UPDATE releases SET title='Changed',revision=revision+1",
        "UPDATE releases SET updated_at=updated_at+1,revision=revision+1",
        "UPDATE releases SET version='0.6.0',version_key='0.6.0',revision=revision+1",
        "UPDATE releases SET published_at=published_at+1,revision=revision+1",
    ] {
        assert!(sqlx::query(sql).execute(&t.state.database).await.is_err());
    }
}
#[tokio::test]
async fn append_races_serialize_conflicts_and_publication_rules() {
    let t = TestApp::new().await;
    let r = t.create("0.5.0").await;
    t.upload(&r, "windows-x86_64", "installer.exe").await;
    let metadata = || UploadMetadata {
        filename: "updater.exe".into(),
        target: "windows-x86_64".into(),
        file_type: None,
        distribution_role: Some("updater".into()),
    };
    let mut unsigned = t
        .state
        .artifacts
        .begin(&t.actor, &t.app, &r.id, metadata())
        .await
        .unwrap();
    unsigned.write(b"unsigned").await.unwrap();
    t.publish(&r).await;
    assert!(matches!(
        t.state.artifacts.complete(&t.actor, unsigned).await,
        Err(ServiceError::UpdaterSignatureRequired)
    ));
    let mut one = t
        .state
        .artifacts
        .begin(&t.actor, &t.app, &r.id, metadata())
        .await
        .unwrap();
    let mut two = t
        .state
        .artifacts
        .begin(&t.actor, &t.app, &r.id, metadata())
        .await
        .unwrap();
    for pending in [&mut one, &mut two] {
        pending.write(b"signed").await.unwrap();
        pending.set_signature(signature().as_bytes()).unwrap();
    }
    let before = t.state.releases.get(&t.app, &r.id).await.unwrap();
    let (one, two) = tokio::join!(
        t.state.artifacts.complete(&t.actor, one),
        t.state.artifacts.complete(&t.actor, two)
    );
    assert!(matches!(
        (&one, &two),
        (Ok(_), Err(ServiceError::ArtifactConflict)) | (Err(ServiceError::ArtifactConflict), Ok(_))
    ));
    assert_eq!(
        t.state.releases.get(&t.app, &r.id).await.unwrap().revision,
        before.revision + 1
    );
    assert_eq!(
        t.state.artifacts.list(&t.app, &r.id).await.unwrap().len(),
        2
    );
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&t.state.database)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn published_installer_addition_preserves_filtered_latest_and_can_follow_updater() {
    let t = TestApp::new().await;
    let r = t.create("0.5.0").await;
    response_json(
        t.upload_form(
            &r,
            "windows-x86_64",
            "update.exe",
            Some("updater"),
            Some(&signature()),
        )
        .await,
        201,
    )
    .await;
    t.publish(&r).await;
    // A published release initially containing only an updater can gain an installer.
    response_json(
        t.upload_form(
            &r,
            "windows-x86_64",
            "installer.exe",
            Some("installer"),
            None,
        )
        .await,
        201,
    )
    .await;
    response_json(
        t.upload_form(&r, "linux-x86_64", "installer.deb", None, None)
            .await,
        201,
    )
    .await;
    response_json(
        t.upload_form(&r, "linux-x86_64", "installer.rpm", None, None)
            .await,
        201,
    )
    .await;
    response_json(
        t.upload_form(&r, "windows-x86_64", "installer.msi", None, None)
            .await,
        201,
    )
    .await;
}
#[tokio::test]
async fn failed_published_append_rolls_back_and_reclaims_file_and_rechecks_live_access() {
    use veruna_releases::domain::application::Status;
    let t = TestApp::new().await;
    let r = t.create("0.5.0").await;
    let a = t.upload(&r, "windows-x86_64", "setup.exe").await;
    t.publish(&r).await;
    let before = t.state.releases.get(&t.app, &r.id).await.unwrap();
    sqlx::query("CREATE TRIGGER fail_revision BEFORE UPDATE OF revision ON releases BEGIN SELECT RAISE(ABORT,'test rollback'); END").execute(&t.state.database).await.unwrap();
    response_json(
        t.upload_form(
            &r,
            "windows-x86_64",
            "update.exe",
            Some("updater"),
            Some(&signature()),
        )
        .await,
        500,
    )
    .await;
    assert_eq!(t.state.releases.get(&t.app, &r.id).await.unwrap(), before);
    assert_eq!(
        t.state.artifacts.list(&t.app, &r.id).await.unwrap(),
        vec![a]
    );
    for _ in 0..100 {
        if std::fs::read_dir(&t.config.storage.directory)
            .unwrap()
            .count()
            == 2
            && std::fs::read_dir(t.config.storage.temporary_directory())
                .unwrap()
                .count()
                == 0
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(
        std::fs::read_dir(&t.config.storage.directory)
            .unwrap()
            .count(),
        2
    ); // .tmp and original object
    assert_eq!(
        std::fs::read_dir(t.config.storage.temporary_directory())
            .unwrap()
            .count(),
        0
    );
    sqlx::query("DROP TRIGGER fail_revision")
        .execute(&t.state.database)
        .await
        .unwrap();
    let metadata = || UploadMetadata {
        filename: "update.exe".into(),
        target: "windows-x86_64".into(),
        file_type: None,
        distribution_role: Some("updater".into()),
    };
    let mut pending = t
        .state
        .artifacts
        .begin(&t.actor, &t.app, &r.id, metadata())
        .await
        .unwrap();
    pending.set_signature(signature().as_bytes()).unwrap();
    pending.write(b"package").await.unwrap();
    t.state
        .applications
        .set_status(&t.actor, &t.app, Status::Archived)
        .await
        .unwrap();
    assert!(matches!(
        t.state.artifacts.complete(&t.actor, pending).await,
        Err(ServiceError::ReleaseLocked)
    ));
    t.state
        .applications
        .set_status(&t.actor, &t.app, Status::Active)
        .await
        .unwrap();
    let mut pending = t
        .state
        .artifacts
        .begin(&t.actor, &t.app, &r.id, metadata())
        .await
        .unwrap();
    pending.set_signature(signature().as_bytes()).unwrap();
    pending.write(b"package").await.unwrap();
    let path = format!("{}/artifacts", t.release_path(&r));
    assert_eq!(
        t.request("POST", &path, Body::empty(), None, false)
            .await
            .status(),
        401
    );
    assert_eq!(
        t.request("POST", &path, Body::empty(), Some(&t.token), false)
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
        t.request("POST", &path, Body::empty(), Some(&operator), true)
            .await
            .status(),
        403
    );
    t.state.sessions.logout(&t.actor).await.unwrap();
    assert!(matches!(
        t.state.artifacts.complete(&t.actor, pending).await,
        Err(ServiceError::Unauthorized)
    ));
    assert_eq!(t.state.releases.get(&t.app, &r.id).await.unwrap(), before);
}
