use axum::{Router, body::Body, http::Request, response::Response};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tempfile::{TempDir, tempdir};
use tower::ServiceExt;
use veruna_releases::{
    application_state,
    config::AppConfig,
    domain::{
        application::{ApplicationChanges, NewApplication, Status, Visibility},
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
    async fn command(&self, r: &Release, channel: &str) -> PublicationCommand {
        let current = self.state.releases.get(&self.app, &r.id).await.unwrap();
        let channels = self.state.publishing.channels(&self.app).await.unwrap();
        let channel = channels.into_iter().find(|c| c.name == channel).unwrap();
        PublicationCommand {
            channel: channel.name,
            expected_channel_revision: channel.revision,
            expected_release_revision: current.revision,
        }
    }
    async fn publish(&self, r: &Release, channel: &str) {
        self.state
            .publishing
            .publish(
                &self.actor,
                &self.app,
                &r.id,
                self.command(r, channel).await,
            )
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
    json!({"channel":c.channel,"expected_channel_revision":c.expected_channel_revision,"expected_release_revision":c.expected_release_revision})
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
            json!({"channel":"stable"}),
            200,
        )
        .await;
    assert_eq!(check["validation"]["state"], "ready");
    assert_eq!(check["channel"]["current_version"], Value::Null);
    let command = command_json(t.command(&r, "stable").await);
    let path = format!("{}/publish", t.release_path(&r));
    let first = t.call("POST", &path, command.clone(), 200).await;
    let published = t.state.releases.get(&t.app, &r.id).await.unwrap();
    assert_eq!(published.status, ReleaseStatus::Published);
    assert!(published.published_at.is_some());
    assert_eq!(published.channels, vec!["stable"]);
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
                    file_type: None
                }
            )
            .await
            .is_err()
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
        "UPDATE publication_history SET action='channel_promoted'",
        "DELETE FROM channel_releases",
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
            json!({"channel":"stable"}),
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
        .validate(&t.actor, &t.app, &r.id, "stable")
        .await
        .unwrap();
    assert_eq!(validation.validation.errors[0].code, "no_artifacts");
    t.call(
        "POST",
        &format!("{}/publish", t.release_path(&r)),
        command_json(t.command(&r, "stable").await),
        422,
    )
    .await;
    let a = t.upload(&r, "linux-x86_64", "package.deb").await;
    t.corrupt(&a, b"damaged").await;
    let validation = t
        .state
        .publishing
        .validate(&t.actor, &t.app, &r.id, "stable")
        .await
        .unwrap();
    assert_eq!(validation.validation.errors[0].code, "checksum_mismatch");
    let result = t
        .call(
            "POST",
            &format!("{}/publish", t.release_path(&r)),
            command_json(t.command(&r, "stable").await),
            422,
        )
        .await;
    assert_eq!(result["validation"]["errors"][0]["artifact_id"], a.id);
    t.corrupt(&a, b"x").await;
    let validation = t
        .state
        .publishing
        .validate(&t.actor, &t.app, &r.id, "stable")
        .await
        .unwrap();
    assert_eq!(validation.validation.errors[0].code, "file_unavailable");
    tokio::fs::remove_file(t.config.storage.directory.join(&a.storage_key))
        .await
        .unwrap();
    assert_eq!(
        t.state
            .publishing
            .validate(&t.actor, &t.app, &r.id, "stable")
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
            .validate(&t.actor, &t.app, &r.id, "stable")
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
            .publish(&t.actor, &t.app, &r.id, t.command(&r, "stable").await)
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
async fn channels_support_beta_promotion_multiple_memberships_rollback_history_and_semver() {
    let t = TestApp::new().await;
    let channels = t
        .call("GET", &format!("{}/channels", t.base()), Value::Null, 200)
        .await;
    assert_eq!(channels.as_array().unwrap().len(), 2);
    assert!(
        channels
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["current_release_id"].is_null())
    );
    let preview = t.create("2.0.0-rc.1").await;
    t.upload(&preview, "any", "preview.zip").await;
    t.publish(&preview, "beta").await;
    assert!(matches!(
        t.state
            .publishing
            .publish(
                &t.actor,
                &t.app,
                &preview.id,
                t.command(&preview, "stable").await
            )
            .await,
        Err(ServiceError::PublicationBlocked(_))
    ));
    let older = t.create("1.9.0").await;
    t.upload(&older, "any", "old.zip").await;
    t.publish(&older, "beta").await;
    let date = t
        .state
        .releases
        .get(&t.app, &older.id)
        .await
        .unwrap()
        .published_at;
    t.publish(&older, "stable").await;
    let newer = t.create("1.10.0+build.9").await;
    t.upload(&newer, "any", "new.zip").await;
    t.publish(&newer, "stable").await;
    let c = t.command(&older, "stable").await;
    t.call("PUT",&format!("{}/channels/stable/current",t.base()),json!({"release_id":older.id,"expected_channel_revision":c.expected_channel_revision,"expected_release_revision":c.expected_release_revision}),200).await;
    let channels = t.state.publishing.channels(&t.app).await.unwrap();
    let stable = channels.iter().find(|c| c.name == "stable").unwrap();
    assert_eq!(stable.current_version.as_deref(), Some("1.9.0"));
    assert_eq!(stable.release_count, 2);
    assert_eq!(
        t.state
            .releases
            .get(&t.app, &older.id)
            .await
            .unwrap()
            .published_at,
        date
    );
    assert_eq!(
        t.state
            .artifacts
            .list(&t.app, &older.id)
            .await
            .unwrap()
            .len(),
        1
    );
    let events = t.state.publishing.history(&t.app, 50, None).await.unwrap();
    assert_eq!(events.len(), 5);
    assert_eq!(events[0].action, "channel_rolled_back");
    assert_eq!(
        events[0].previous_version.as_deref(),
        Some("1.10.0+build.9")
    );
    assert_eq!(events[0].new_version, "1.9.0");
    assert_eq!(events[2].action, "channel_promoted");
    let paged = t
        .state
        .publishing
        .history(&t.app, 2, Some(events[1].id))
        .await
        .unwrap();
    assert_eq!(paged[0].id, events[2].id);
    let draft = t.create("3.0.0").await;
    assert!(matches!(
        t.state
            .publishing
            .switch(
                &t.actor,
                &t.app,
                &draft.id,
                t.command(&draft, "stable").await
            )
            .await,
        Err(ServiceError::ReleaseLocked)
    ));
    // Corruption after publication cannot be used as a rollback destination.
    let a = t
        .state
        .artifacts
        .list(&t.app, &newer.id)
        .await
        .unwrap()
        .remove(0);
    t.corrupt(&a, b"damaged").await;
    assert!(matches!(
        t.state
            .publishing
            .switch(
                &t.actor,
                &t.app,
                &newer.id,
                t.command(&newer, "stable").await
            )
            .await,
        Err(ServiceError::PublicationBlocked(_))
    ));
}
#[tokio::test]
async fn channel_ownership_and_optimistic_revisions_reject_stale_and_parallel_commands() {
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
    let ca = t.command(&a, "stable").await;
    let cb = t.command(&b, "stable").await;
    let (one, two) = tokio::join!(
        t.state.publishing.publish(&t.actor, &t.app, &a.id, ca),
        t.state.publishing.publish(&second, &t.app, &b.id, cb)
    );
    assert!(matches!(
        (&one, &two),
        (Ok(_), Err(ServiceError::PublicationConflict))
            | (Err(ServiceError::PublicationConflict), Ok(_))
    ));
    assert_eq!(
        t.state
            .publishing
            .history(&t.app, 50, None)
            .await
            .unwrap()
            .len(),
        1
    );
    let draft = if one.is_err() { &a } else { &b };
    let stale = t.command(draft, "stable").await;
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
                    channel: "stable".into(),
                    expected_channel_revision: 0,
                    expected_release_revision: 1
                }
            )
            .await,
        Err(ServiceError::NotFound)
    ));
    let channel = t
        .state
        .publishing
        .channels(&foreign.id)
        .await
        .unwrap()
        .remove(0);
    assert!(sqlx::query("INSERT INTO channel_releases(channel_id,release_id,application_id,added_at) VALUES(?,?,?,1)").bind(&channel.id).bind(&a.id).bind(&foreign.id).execute(&t.state.database).await.is_err());
    assert!(
        sqlx::query("UPDATE release_channels SET current_release_id=? WHERE id=?")
            .bind(&a.id)
            .bind(&channel.id)
            .execute(&t.state.database)
            .await
            .is_err()
    );
}
#[tokio::test]
async fn public_manifest_honors_current_pointer_history_limits_etag_and_privacy() {
    let t = TestApp::new().await;
    t.call("GET", "/product/manifest.json", Value::Null, 404)
        .await;
    let r = t.create("1.9.0+ci.1").await;
    let a = t.upload(&r, "windows-x86_64", "setup.exe").await;
    t.publish(&r, "stable").await;
    let newer = t.create("1.10.0").await;
    t.upload(&newer, "linux-x86_64", "package.deb").await;
    t.publish(&newer, "stable").await;
    let beta = t.create("2.0.0-beta.1").await;
    t.upload(&beta, "any", "preview.zip").await;
    t.publish(&beta, "beta").await;
    t.create("99.0.0").await;
    t.state
        .publishing
        .switch(&t.actor, &t.app, &r.id, t.command(&r, "stable").await)
        .await
        .unwrap();
    let mut manifest = t
        .call("GET", "/product/manifest.json", Value::Null, 200)
        .await;
    assert_eq!(manifest["schema_version"], 1);
    assert_eq!(manifest["app"], "product");
    assert_eq!(manifest["latest"], "1.9.0+ci.1");
    assert_eq!(manifest["releases"].as_array().unwrap().len(), 2);
    assert_eq!(manifest["releases"][0]["version"], manifest["latest"]);
    let metadata = &manifest["releases"][0]["artifacts"][0];
    assert_eq!(metadata["id"], a.id);
    assert!(
        metadata["download_url"]
            .as_str()
            .unwrap()
            .contains("/artifacts/")
    );
    assert!(metadata.get("storage_key").is_none());
    let date = manifest["releases"][0]["published_at"].as_str().unwrap();
    assert!(date.ends_with('Z'));
    assert!(chrono::DateTime::parse_from_rfc3339(date).is_ok());
    assert_eq!(
        t.call("GET", "/product/manifest.json?limit=1", Value::Null, 200)
            .await["releases"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        t.call(
            "GET",
            "/product/manifest.json?channel=beta",
            Value::Null,
            200
        )
        .await["latest"],
        "2.0.0-beta.1"
    );
    let response = t
        .request("GET", "/product/manifest.json", Body::empty(), None, false)
        .await;
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-type"], "application/json");
    let etag = response.headers()["etag"].clone();
    let conditional = || {
        Request::builder()
            .uri("/product/manifest.json")
            .header("if-none-match", &etag)
            .body(Body::empty())
            .unwrap()
    };
    let response = t.router.clone().oneshot(conditional()).await.unwrap();
    assert_eq!(response.status(), 304);
    assert!(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .is_empty()
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
    assert_eq!(
        t.router
            .clone()
            .oneshot(conditional())
            .await
            .unwrap()
            .status(),
        404
    );
    // Private previews omit public download links while preserving release metadata.
    for release in manifest["releases"].as_array_mut().unwrap() {
        for artifact in release["artifacts"].as_array_mut().unwrap() {
            artifact.as_object_mut().unwrap().remove("download_url");
        }
    }
    assert_eq!(
        t.call(
            "GET",
            &format!("{}/manifests/stable/preview", t.base()),
            Value::Null,
            200
        )
        .await,
        manifest
    );
    t.state
        .applications
        .set_status(&t.actor, &t.app, Status::Archived)
        .await
        .unwrap();
    assert_eq!(
        t.router
            .clone()
            .oneshot(conditional())
            .await
            .unwrap()
            .status(),
        404
    );
    t.call("GET", "/product/manifest.json?limit=101", Value::Null, 400)
        .await;
    t.call(
        "GET",
        "/product/manifest.json?unknown=yes",
        Value::Null,
        400,
    )
    .await;
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
    t.corrupt(&a, b"truncated").await;
    t.call("GET", "/product/manifest.json", Value::Null, 503)
        .await;
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
        .validate(&t.actor, &t.app, &r.id, "stable")
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
    t.publish(&r, "stable").await;
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
            json!({"channel":"stable","target":"windows-x86_64","current_version":"1.0.0"}),
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
                json!({"channel":"stable","target":"windows-x86_64","current_version":version}),
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
            json!({"channel":"stable","target":"darwin-aarch64","current_version":"1.0.0"}),
            200
        )
        .await["reason"],
        "updater_package_missing"
    );
    t.call(
        "POST",
        &preview_path,
        json!({"channel":"stable","target":"windows-x86_64","current_version":"garbage"}),
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
            json!({"channel":"stable","target":"windows-x86_64","current_version":"1.0.0"}),
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
    t.publish(&r, "stable").await;
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
        ("linux-x86_64", "deb"),
    ] {
        a.target = target.into();
        a.file_type = format.into();
        assert!(matches!(
            updater::select(r.clone(), vec![a.clone()], target, "1.0.0").unwrap(),
            UpdateDecision::MissingPackage
        ));
    }
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
        ("GET", format!("{}/channels", t.base()), true),
        ("GET", format!("{}/publication-history", t.base()), true),
        (
            "POST",
            format!("{}/validate-publication", t.release_path(&r)),
            false,
        ),
        ("POST", format!("{}/publish", t.release_path(&r)), false),
        (
            "PUT",
            format!("{}/channels/stable/current", t.base()),
            false,
        ),
        (
            "GET",
            format!("{}/manifests/stable/preview", t.base()),
            false,
        ),
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
                    r#"{"channel":"stable","target":"windows-x86_64","current_version":"1.0.0"}"#
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
                    channel: "stable".into(),
                    expected_channel_revision: 0,
                    expected_release_revision: 1
                }
            )
            .await,
        Err(ServiceError::Unauthorized)
    ));
}

#[tokio::test]
async fn ambiguous_updater_blocks_publication_and_inflight_upload_cannot_change_published_files() {
    let t = TestApp::new().await;
    let r = t.create("1.0.0").await;
    let one = t.upload(&r, "windows-x86_64", "setup.exe").await;
    let two = t.upload(&r, "windows-x86_64", "setup.msi").await;
    for a in [&one, &two] {
        t.state
            .artifacts
            .set_updater(&t.actor, &t.app, &r.id, &a.id, Some("updater"), None)
            .await
            .unwrap();
    }
    let check = t
        .state
        .publishing
        .validate(&t.actor, &t.app, &r.id, "stable")
        .await
        .unwrap();
    assert!(
        check
            .validation
            .errors
            .iter()
            .any(|e| e.code == "ambiguous_updater")
    );
    t.state
        .artifacts
        .set_updater(&t.actor, &t.app, &r.id, &two.id, Some("installer"), None)
        .await
        .unwrap();
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
            },
        )
        .await
        .unwrap();
    pending.write(b"pending").await.unwrap();
    t.publish(&r, "stable").await;
    assert!(matches!(
        t.state.artifacts.complete(&t.actor, pending).await,
        Err(ServiceError::ReleaseLocked)
    ));
    assert_eq!(
        t.state.artifacts.list(&t.app, &r.id).await.unwrap().len(),
        2
    );
    let preview = t
        .state
        .updater
        .preview(&t.app, "stable", "windows-x86_64", "0.0.0")
        .await
        .unwrap();
    assert_eq!(preview.reason, "signature_missing");
}

#[tokio::test]
async fn history_failure_rolls_back_publication_membership_and_current_pointer() {
    let t = TestApp::new().await;
    let r = t.create("1.0.0").await;
    t.upload(&r, "any", "package.zip").await;
    sqlx::query("CREATE TRIGGER fail_publication_history BEFORE INSERT ON publication_history BEGIN SELECT RAISE(ABORT,'test failure'); END").execute(&t.state.database).await.unwrap();
    assert!(
        t.state
            .publishing
            .publish(&t.actor, &t.app, &r.id, t.command(&r, "stable").await)
            .await
            .is_err()
    );
    assert_eq!(
        t.state.releases.get(&t.app, &r.id).await.unwrap().status,
        ReleaseStatus::Draft
    );
    let channels = t.state.publishing.channels(&t.app).await.unwrap();
    assert!(
        channels
            .iter()
            .all(|c| c.current_release_id.is_none() && c.release_count == 0 && c.revision == 0)
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
    t.publish(&r, "stable").await;
}
