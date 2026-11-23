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
    product: String,
}
impl TestApp {
    async fn new() -> Self {
        let t = Self::unconfigured().await;
        t.state
            .applications
            .set_standalone_settings(
                &t.actor,
                &t.app,
                Some(Some(&t.product)),
                Some(Some(PUBLIC_KEY)),
            )
            .await
            .unwrap();
        t
    }
    async fn unconfigured() -> Self {
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
        let product = uuid::Uuid::new_v4().to_string();
        let router = http::router(state.clone());
        Self {
            _dir: dir,
            config,
            state,
            router,
            actor,
            token: login.token,
            app,
            product,
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
                    sign(&metadata(&self.product, &r.version.to_string(), target)).as_bytes(),
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
        self.multipart_metadata(
            r,
            UploadMetadata {
                filename: filename.into(),
                target: target.into(),
                file_type: None,
                distribution_role: None,
            },
            jws,
            truncated,
        )
        .await
    }
    async fn multipart_metadata(
        &self,
        r: &Release,
        metadata: UploadMetadata,
        jws: Option<&str>,
        truncated: bool,
    ) -> Response {
        let mut form = format!(
            "--BOUNDARY\r\nContent-Disposition: form-data; name=\"target\"\r\n\r\n{}\r\n",
            metadata.target
        );
        for (name, value) in [
            ("file_type", metadata.file_type),
            ("distribution_role", metadata.distribution_role),
        ] {
            if let Some(value) = value {
                form.push_str(&format!("--BOUNDARY\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"));
            }
        }
        form.push_str(&format!(
            "--BOUNDARY\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{}\"\r\nContent-Type: application/octet-stream\r\n\r\n{}\r\n",
            metadata.filename,
            std::str::from_utf8(CONTENT).unwrap()
        ));
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
async fn signed_executables_use_canonical_target_independently_of_filename_format() {
    let t = TestApp::new().await;
    let r = t.create("0.7.0").await;
    for (target, filename) in [
        ("linux-x86_64", "product-0.7.0-linux-x86_64"),
        ("linux-aarch64", "product.aarch64"),
        ("darwin-aarch64", "product-0.7.0-darwin-aarch64.tool"),
        ("windows-x86_64", "product-0.7.0-windows-x86_64.exe"),
    ] {
        let upload_metadata = || UploadMetadata {
            filename: filename.into(),
            target: target.into(),
            file_type: None,
            distribution_role: Some("executable".into()),
        };
        // CPU aliases and another ABI cannot be smuggled in through signed fields.
        for arch in ["x64", "arm64", "unknown"] {
            let mut claims = metadata(&t.product, "0.7.0", target);
            claims["arch"] = json!(arch);
            assert_eq!(
                t.multipart_metadata(&r, upload_metadata(), Some(&sign(&claims)), false)
                    .await
                    .status(),
                422
            );
        }
        let signature = sign(&metadata(&t.product, "0.7.0", target));
        let response = t
            .multipart_metadata(&r, upload_metadata(), Some(&signature), false)
            .await;
        let status = response.status();
        let artifact = body(response).await;
        assert_eq!(status, 201, "{target}: {artifact}");
        assert_eq!(artifact["target"], target);
        assert_eq!(artifact["filename"], filename);
        if filename == "product-0.7.0-linux-x86_64" {
            assert_eq!(artifact["file_type"], "bin");
        }
        assert_eq!(artifact["distribution_role"], "executable");
        assert_eq!(artifact["standalone_signature_present"], true);
        assert_eq!(artifact["signature_present"], false);
        if target == "windows-x86_64" {
            for role in ["installer", "executable"] {
                let changed = t
                    .state
                    .artifacts
                    .set_updater(
                        &t.actor,
                        &t.app,
                        &r.id,
                        artifact["id"].as_str().unwrap(),
                        Some(role),
                        None,
                    )
                    .await
                    .unwrap();
                assert_eq!(
                    changed.standalone_signature.as_deref(),
                    Some(signature.as_str())
                );
            }
        }
        assert!(matches!(
            t.state
                .artifacts
                .set_updater(
                    &t.actor,
                    &t.app,
                    &r.id,
                    artifact["id"].as_str().unwrap(),
                    Some("archive"),
                    None
                )
                .await,
            Err(ServiceError::InvalidStandalone)
        ));
    }
    // The one-signed-file-per-target rule spans both classifications and formats.
    let duplicate = t
        .multipart_metadata(
            &r,
            UploadMetadata {
                filename: "legacy.bin".into(),
                target: "linux-x86_64".into(),
                file_type: None,
                distribution_role: Some("installer".into()),
            },
            Some(&sign(&metadata(&t.product, "0.7.0", "linux-x86_64"))),
            false,
        )
        .await;
    assert_eq!(duplicate.status(), 409);
    t.publish(&r).await.unwrap();
    for target in [
        "linux-x86_64",
        "linux-aarch64",
        "darwin-aarch64",
        "windows-x86_64",
    ] {
        let response = t.update(target, "0.6.0").await;
        assert_eq!(response.status(), 200);
        assert_eq!(body(response).await["version"], "0.7.0");
        assert_eq!(t.update(target, "0.7.0").await.status(), 204);
        assert_eq!(t.update(target, "0.8.0").await.status(), 204);
    }
    let manifest = body(t.get("/product/manifest.json").await).await;
    assert_eq!(
        manifest["latest_by_type"]["executable"]["linux-x86_64"]["version"],
        "0.7.0"
    );
    assert_eq!(manifest["latest_by_target"], json!({}));
    t.assert_storage_count(4).await;
}

#[tokio::test]
async fn executable_migration_preserves_signatures_counters_and_database_guards() {
    let dir = tempdir().unwrap();
    let old_migrations = dir.path().join("old-migrations");
    std::fs::create_dir(&old_migrations).unwrap();
    for entry in std::fs::read_dir("migrations").unwrap() {
        let entry = entry.unwrap();
        if entry.file_name().to_str().unwrap() < "0015" {
            std::fs::copy(entry.path(), old_migrations.join(entry.file_name())).unwrap();
        }
    }
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(sqlx::sqlite::SqliteConnectOptions::new().foreign_keys(true))
        .await
        .unwrap();
    sqlx::migrate::Migrator::new(old_migrations.as_path())
        .await
        .unwrap()
        .run(&pool)
        .await
        .unwrap();
    let app = uuid::Uuid::new_v4().to_string();
    let product = uuid::Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO applications(id,slug,name,name_search,product_id,standalone_public_key) VALUES(?,'product','Product','product',?,?)")
        .bind(&app).bind(&product).bind(PUBLIC_KEY).execute(&pool).await.unwrap();
    let snapshot = "SELECT json_object('id',id,'release',release_id,'filename',filename,'target',target,'format',file_type,'role',distribution_role,'signature',standalone_signature,'size',size_bytes,'sha256',sha256,'key',storage_key,'created',created_at) FROM artifacts ORDER BY id";
    let mut records = Vec::new();
    for (version, status) in [("0.5.0", "published"), ("0.6.0", "draft")] {
        let release = uuid::Uuid::new_v4().to_string();
        let artifact = uuid::Uuid::new_v4().to_string();
        let storage_key = format!("{}.blob", uuid::Uuid::new_v4().simple());
        let signature = sign(&metadata(&product, version, "linux-x86_64"));
        sqlx::query("INSERT INTO releases(id,application_id,version,version_key,status,created_at,updated_at,published_at) VALUES(?,?,?,?,?,1,1,?)")
            .bind(&release).bind(&app).bind(version).bind(version).bind(status).bind((status == "published").then_some(1_i64)).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO artifacts(id,release_id,filename,target,file_type,size_bytes,sha256,storage_key,created_at,standalone_signature) VALUES(?,?,'product','linux-x86_64','bin',?,?,?,?,?)")
            .bind(&artifact).bind(&release).bind(CONTENT.len() as i64).bind(format!("{:x}",Sha256::digest(CONTENT))).bind(&storage_key).bind(1_i64).bind(&signature).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO artifact_download_counts VALUES(?,17)")
            .bind(&artifact)
            .execute(&pool)
            .await
            .unwrap();
        records.push((release, artifact));
    }
    let before: Vec<String> = sqlx::query_scalar(snapshot).fetch_all(&pool).await.unwrap();
    let counters: Vec<(String, i64)> =
        sqlx::query_as("SELECT * FROM artifact_download_counts ORDER BY artifact_id")
            .fetch_all(&pool)
            .await
            .unwrap();
    sqlx::migrate!().run(&pool).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, String>(snapshot)
            .fetch_all(&pool)
            .await
            .unwrap(),
        before
    );
    assert_eq!(
        sqlx::query_as::<_, (String, i64)>(
            "SELECT * FROM artifact_download_counts ORDER BY artifact_id"
        )
        .fetch_all(&pool)
        .await
        .unwrap(),
        counters
    );
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&pool)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        sqlx::query("UPDATE artifacts SET standalone_signature=NULL WHERE id=?")
            .bind(&records[0].1)
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE applications SET standalone_public_key=NULL WHERE id=?")
            .bind(&app)
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE artifacts SET distribution_role='archive' WHERE id=?")
            .bind(&records[1].1)
            .execute(&pool)
            .await
            .is_err()
    );
    // A signed legacy installer can become an executable in a draft without losing its JWS.
    sqlx::query("UPDATE artifacts SET distribution_role='executable' WHERE id=?")
        .bind(&records[1].1)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT standalone_signature FROM artifacts WHERE id=?")
            .bind(&records[1].1)
            .fetch_one(&pool)
            .await
            .unwrap(),
        sign(&metadata(&product, "0.6.0", "linux-x86_64"))
    );
}

#[tokio::test]
async fn identity_migration_preserves_legacy_published_and_draft_jws_byte_for_byte() {
    use veruna_releases::repositories::{
        applications::ApplicationsRepository, artifact::ArtifactsRepository,
        release::ReleasesRepository,
    };
    let dir = tempdir().unwrap();
    let mut config = AppConfig::default();
    config.database.path = dir.path().join("legacy.sqlite");
    config.storage.directory = dir.path().join("objects");
    config.storage.icons_directory = dir.path().join("icons");
    let migrations = dir.path().join("legacy-migrations");
    std::fs::create_dir(&migrations).unwrap();
    for entry in std::fs::read_dir("migrations").unwrap() {
        let entry = entry.unwrap();
        if entry.file_name().to_str().unwrap() < "0011" {
            std::fs::copy(entry.path(), migrations.join(entry.file_name())).unwrap();
        }
    }
    let pool = sqlx::SqlitePool::connect_with(
        sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&config.database.path)
            .create_if_missing(true)
            .foreign_keys(true),
    )
    .await
    .unwrap();
    sqlx::migrate::Migrator::new(migrations.as_path())
        .await
        .unwrap()
        .run(&pool)
        .await
        .unwrap();
    std::fs::create_dir_all(&config.storage.directory).unwrap();
    let mut legacy = Vec::new();
    for (index, status, signed) in [
        (0, "published", true),
        (1, "draft", true),
        (2, "published", false),
    ] {
        let app = uuid::Uuid::new_v4().to_string();
        let release = uuid::Uuid::new_v4().to_string();
        let artifact = uuid::Uuid::new_v4().to_string();
        let storage_key = format!("{}.blob", uuid::Uuid::new_v4().simple());
        let seed = [index + 1; 32];
        let key = URL_SAFE_NO_PAD.encode(
            ed25519_dalek::SigningKey::from_bytes(&seed)
                .verifying_key()
                .as_bytes(),
        );
        let mut header = Header::new(Algorithm::EdDSA);
        header.typ = Some(standalone::MEDIA_TYPE.into());
        if index == 1 {
            header.kid = standalone::key_id(&key);
        }
        let signature =
            signed.then(|| sign_header(&metadata(&app, "0.5.0", "windows-x86_64"), &header, &seed));
        sqlx::query("INSERT INTO applications(id,slug,name,name_search,visibility,standalone_public_key) VALUES(?,?,'Legacy','legacy','public',?)")
            .bind(&app).bind(format!("legacy-{index}")).bind(&key).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO releases(id,application_id,version,version_key,status,created_at,updated_at,published_at) VALUES(?,?,'0.5.0','0.5.0',?,1,1,?)")
            .bind(&release).bind(&app).bind(status).bind((status == "published").then_some(1_i64)).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO artifacts(id,release_id,filename,target,file_type,size_bytes,sha256,storage_key,created_at,standalone_signature) VALUES(?,?,'product.exe','windows-x86_64','exe',?,?,?,?,?)")
            .bind(&artifact).bind(&release).bind(CONTENT.len() as i64).bind(format!("{:x}", Sha256::digest(CONTENT))).bind(&storage_key).bind(1_i64).bind(&signature).execute(&pool).await.unwrap();
        std::fs::write(config.storage.directory.join(&storage_key), CONTENT).unwrap();
        legacy.push((app, release, artifact, storage_key, key, signature));
    }
    let apps_before: Vec<(String, String, String)> =
        sqlx::query_as("SELECT id,slug,standalone_public_key FROM applications ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
    // Snapshot the historical schema directly; current repositories also read
    // counters introduced after this fixture's migration version.
    let release_snapshot = "SELECT json_object('id',id,'application_id',application_id,'version',version,'version_key',version_key,'title',title,'release_notes',release_notes,'status',status,'revision',revision,'created_at',created_at,'updated_at',updated_at,'published_at',published_at) FROM releases ORDER BY id";
    let artifact_snapshot = "SELECT json_object('id',id,'release_id',release_id,'filename',filename,'target',target,'file_type',file_type,'size_bytes',size_bytes,'sha256',sha256,'storage_key',storage_key,'created_at',created_at,'distribution_role',distribution_role,'updater_signature',updater_signature,'standalone_signature',standalone_signature) FROM artifacts ORDER BY id";
    let releases_before: Vec<String> = sqlx::query_scalar(release_snapshot)
        .fetch_all(&pool)
        .await
        .unwrap();
    let artifacts_before: Vec<String> = sqlx::query_scalar(artifact_snapshot)
        .fetch_all(&pool)
        .await
        .unwrap();
    pool.close().await;
    let pool = initialize(&config).await.unwrap();
    assert_eq!(
        sqlx::query_as::<_, (String, String, String)>(
            "SELECT id,slug,standalone_public_key FROM applications ORDER BY id"
        )
        .fetch_all(&pool)
        .await
        .unwrap(),
        apps_before
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>(release_snapshot)
            .fetch_all(&pool)
            .await
            .unwrap(),
        releases_before
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>(artifact_snapshot)
            .fetch_all(&pool)
            .await
            .unwrap(),
        artifacts_before
    );
    for (app, release, artifact, storage_key, key, signature) in &legacy {
        let p = ApplicationsRepository::new(pool.clone())
            .get(app)
            .await
            .unwrap();
        let r = ReleasesRepository::new(pool.clone())
            .get(app, release)
            .await
            .unwrap();
        let a = ArtifactsRepository::new(pool.clone())
            .get(app, release, artifact)
            .await
            .unwrap();
        assert_eq!(r.download_count, 0);
        assert_eq!(a.download_count, 0);
        assert_eq!(
            p.product_id.as_deref(),
            signature.as_ref().map(|_| app.as_str())
        );
        assert_eq!(p.product_id_locked, signature.is_some());
        assert_eq!(p.standalone_public_key.as_deref(), Some(key.as_str()));
        assert_eq!(
            std::fs::read(config.storage.directory.join(storage_key)).unwrap(),
            CONTENT
        );
        if let Some(signature) = signature {
            assert_eq!(
                a.standalone_signature.as_ref().unwrap().as_bytes(),
                signature.as_bytes()
            );
            assert!(standalone::verify(signature, &p, &r, &a).is_some());
            assert!(
                sqlx::query("UPDATE applications SET product_id=NULL WHERE id=?")
                    .bind(app)
                    .execute(&pool)
                    .await
                    .is_err()
            );
        }
    }
    let state = application_state(pool.clone(), &config).await.unwrap();
    let update = state
        .standalone_updater
        .update("legacy-0", "windows-x86_64", "0.4.0")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        update.signed_metadata.as_bytes(),
        legacy[0].5.as_ref().unwrap().as_bytes()
    );
    assert!(
        state
            .standalone_updater
            .update("legacy-1", "windows-x86_64", "0.4.0")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        state
            .standalone_updater
            .update("legacy-2", "windows-x86_64", "0.4.0")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&pool)
            .await
            .unwrap()
            .is_empty()
    );
    pool.close().await;
    // SQLx checksums and identity pinning also survive a second startup.
    let pool = initialize(&config).await.unwrap();
    assert_eq!(
        ApplicationsRepository::new(pool.clone())
            .get(&legacy[0].0)
            .await
            .unwrap()
            .product_id
            .as_deref(),
        Some(legacy[0].0.as_str())
    );
    pool.close().await;
}

#[tokio::test]
async fn publisher_signature_works_on_two_independent_servers() {
    // Build/sign before either server exists. Neither local ID is known to the signer.
    let product = uuid::Uuid::new_v4().to_string();
    let mut seed = [0_u8; 32];
    getrandom::fill(&mut seed).unwrap();
    let public_key = URL_SAFE_NO_PAD.encode(
        ed25519_dalek::SigningKey::from_bytes(&seed)
            .verifying_key()
            .as_bytes(),
    );
    let mut header = Header::new(Algorithm::EdDSA);
    header.typ = Some(standalone::MEDIA_TYPE.into());
    header.kid = standalone::key_id(&public_key);
    let signature = sign_header(
        &metadata(&product, "1.2.3", "windows-x86_64"),
        &header,
        &seed,
    );
    let a = TestApp::unconfigured().await;
    let b = TestApp::unconfigured().await;
    assert_ne!(a.app, b.app);
    for t in [&a, &b] {
        assert_ne!(t.app, product);
        let settings = format!("{}/integrations/standalone", t.base());
        assert_eq!(
            t.admin(
                "PUT",
                &settings,
                json!({"product_id":product,"public_key":public_key}),
                Some(&t.token),
                true
            )
            .await
            .status(),
            204
        );
        let r = t.create("1.2.3").await;
        // Matching a server's internal ID must no longer be sufficient.
        assert_eq!(
            t.multipart(
                &r,
                "windows-x86_64",
                Some(&sign_header(
                    &metadata(&t.app, "1.2.3", "windows-x86_64"),
                    &header,
                    &seed
                )),
                false
            )
            .await
            .status(),
            422
        );
        assert_eq!(
            t.multipart(&r, "windows-x86_64", Some(&signature), false)
                .await
                .status(),
            201
        );
        t.publish(&r).await.unwrap();
        let response = t.update("windows-x86_64", "1.0.0").await;
        assert_eq!(response.status(), 200);
        let update = body(response).await;
        assert_eq!(
            update["signed_metadata"].as_str().unwrap().as_bytes(),
            signature.as_bytes()
        );
        let path = url::Url::parse(update["url"].as_str().unwrap()).unwrap();
        let download = t.get(path.path()).await;
        assert_eq!(download.status(), 200);
        assert_eq!(
            download.into_body().collect().await.unwrap().to_bytes(),
            CONTENT
        );
        let app = t.state.applications.get(&t.app).await.unwrap();
        let artifact = t
            .state
            .artifacts
            .list(&t.app, &r.id)
            .await
            .unwrap()
            .remove(0);
        assert!(standalone::verify(&signature, &app, &r, &artifact).is_some());
        let mut wrong_owner = r.clone();
        wrong_owner.application_id = uuid::Uuid::new_v4().to_string();
        assert!(standalone::verify(&signature, &app, &wrong_owner, &artifact).is_none());
    }
}

#[tokio::test]
async fn identity_settings_distinguish_missing_null_and_value_and_roll_back_together() {
    let t = TestApp::unconfigured().await;
    let settings = format!("{}/integrations/standalone", t.base());
    let read = || t.admin("GET", &settings, Value::Null, Some(&t.token), true);
    let initial = body(read().await).await;
    assert_eq!(initial["application_id"], t.app);
    assert!(initial["product_id"].is_null());
    assert!(initial["public_key"].is_null());
    for invalid in [
        "",
        "bad",
        "E81A5470-8414-40A1-8596-71BF674F4568",
        "e81a5470841440a1859671bf674f4568",
        " e81a5470-8414-40a1-8596-71bf674f4568",
    ] {
        assert_eq!(
            t.admin(
                "PUT",
                &settings,
                json!({"product_id":invalid,"public_key":PUBLIC_KEY}),
                Some(&t.token),
                true
            )
            .await
            .status(),
            422
        );
        assert!(
            sqlx::query("UPDATE applications SET product_id=? WHERE id=?")
                .bind(invalid)
                .bind(&t.app)
                .execute(&t.state.database)
                .await
                .is_err()
        );
    }
    for invalid in [json!({"product_id":123}), json!({"extra":true})] {
        assert_eq!(
            t.admin("PUT", &settings, invalid, Some(&t.token), true)
                .await
                .status(),
            400
        );
    }
    // Existing key-only clients retain their contract.
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
        204
    );
    assert_eq!(
        t.admin(
            "PUT",
            &settings,
            json!({"product_id":t.product}),
            Some(&t.token),
            true
        )
        .await
        .status(),
        204
    );
    let configured = body(read().await).await;
    assert_eq!(configured["product_id"], t.product);
    assert_eq!(configured["public_key"], PUBLIC_KEY);
    assert_eq!(
        configured["key_id"],
        standalone::key_id(PUBLIC_KEY).unwrap()
    );
    assert_eq!(
        t.admin("PUT", &settings, json!({}), Some(&t.token), true)
            .await
            .status(),
        204
    );
    assert_eq!(body(read().await).await, configured);
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
        204
    );
    assert_eq!(body(read().await).await["product_id"], t.product);
    assert_eq!(
        t.admin(
            "PUT",
            &settings,
            json!({"product_id":null,"public_key":PUBLIC_KEY}),
            Some(&t.token),
            true
        )
        .await
        .status(),
        204
    );
    let current = body(read().await).await;
    assert!(current["product_id"].is_null());
    assert_eq!(current["public_key"], PUBLIC_KEY);
    // A conflict in one field must roll back the other field too.
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
    t.state
        .applications
        .set_standalone_settings(&t.actor, &other.id, Some(Some(&t.product)), None)
        .await
        .unwrap();
    t.state
        .applications
        .set_status(&t.actor, &other.id, Status::Archived)
        .await
        .unwrap();
    let response = t
        .admin(
            "PUT",
            &settings,
            json!({"product_id":t.product,"public_key":null}),
            Some(&t.token),
            true,
        )
        .await;
    assert_eq!(response.status(), 409);
    assert_eq!(
        body(response).await["error"]["code"],
        "standalone_product_conflict"
    );
    assert_eq!(body(read().await).await, current);
    assert!(
        sqlx::query("UPDATE applications SET product_id=? WHERE id=?")
            .bind(&t.product)
            .bind(&t.app)
            .execute(&t.state.database)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn unsigned_distribution_needs_no_identity_but_first_signature_requires_it() {
    let t = TestApp::unconfigured().await;
    t.state
        .applications
        .set_standalone_key(&t.actor, &t.app, Some(PUBLIC_KEY))
        .await
        .unwrap();
    let r = t.create("0.5.0").await;
    let signature = sign(&metadata(&t.product, "0.5.0", "windows-x86_64"));
    assert_eq!(
        t.multipart(&r, "windows-x86_64", Some(&signature), false)
            .await
            .status(),
        422
    );
    let a = t.upload(&r, "windows-x86_64", false).await;
    assert!(matches!(
        t.state
            .artifacts
            .set_standalone(&t.actor, &t.app, &r.id, &a.id, Some(&signature))
            .await,
        Err(ServiceError::InvalidStandalone)
    ));
    assert!(
        sqlx::query("UPDATE artifacts SET standalone_signature=? WHERE id=?")
            .bind(&signature)
            .bind(&a.id)
            .execute(&t.state.database)
            .await
            .is_err()
    );
    t.publish(&r).await.unwrap();
    assert_eq!(t.update("windows-x86_64", "0.1.0").await.status(), 204);
    let download = t
        .get(&t.state.distribution.artifact_path("product", &r, &a))
        .await;
    assert_eq!(download.status(), 200);
    assert_eq!(
        download.into_body().collect().await.unwrap().to_bytes(),
        CONTENT
    );
    // Unconfigured identity also forbids signed additions to a published release.
    assert_eq!(
        t.multipart(
            &r,
            "linux-x86_64",
            Some(&sign(&metadata(&t.product, "0.5.0", "linux-x86_64"))),
            false
        )
        .await
        .status(),
        422
    );
    assert!(
        t.state
            .applications
            .get(&t.app)
            .await
            .unwrap()
            .product_id
            .is_none()
    );
}

#[tokio::test]
async fn first_signature_permanently_pins_identity_in_service_and_sqlite() {
    let t = TestApp::new().await;
    let r = t.create("0.5.0").await;
    let a = t.upload(&r, "windows-x86_64", true).await;
    let next = uuid::Uuid::new_v4().to_string();
    for published in [false, true] {
        if published {
            t.publish(&r).await.unwrap();
        }
        let response = t
            .admin(
                "PUT",
                &format!("{}/integrations/standalone", t.base()),
                json!({"product_id":next}),
                Some(&t.token),
                true,
            )
            .await;
        assert_eq!(response.status(), 409);
        assert_eq!(
            body(response).await["error"]["code"],
            "standalone_product_conflict"
        );
        for product in [None, Some(next.as_str())] {
            assert!(matches!(
                t.state
                    .applications
                    .set_standalone_settings(&t.actor, &t.app, Some(product), None)
                    .await,
                Err(ServiceError::StandaloneProductConflict)
            ));
            assert!(
                sqlx::query("UPDATE applications SET product_id=? WHERE id=?")
                    .bind(product)
                    .bind(&t.app)
                    .execute(&t.state.database)
                    .await
                    .is_err()
            );
        }
        assert!(
            sqlx::query("UPDATE applications SET product_id_locked=0,product_id=? WHERE id=?")
                .bind(&next)
                .bind(&t.app)
                .execute(&t.state.database)
                .await
                .is_err()
        );
        t.state
            .applications
            .set_standalone_settings(&t.actor, &t.app, Some(Some(&t.product)), None)
            .await
            .unwrap();
    }
    assert_eq!(t.state.artifacts.list(&t.app, &r.id).await.unwrap()[0], a);
    // Also cover an app whose only signed draft is removed.
    let only_draft = TestApp::new().await;
    let r = only_draft.create("0.5.0").await;
    let a = only_draft.upload(&r, "windows-x86_64", false).await;
    let signature = sign(&metadata(&only_draft.product, "0.5.0", "windows-x86_64"));
    only_draft
        .state
        .artifacts
        .set_standalone(
            &only_draft.actor,
            &only_draft.app,
            &r.id,
            &a.id,
            Some(&signature),
        )
        .await
        .unwrap();
    only_draft
        .state
        .artifacts
        .set_standalone(&only_draft.actor, &only_draft.app, &r.id, &a.id, None)
        .await
        .unwrap();
    only_draft
        .state
        .releases
        .delete(&only_draft.actor, &only_draft.app, &r.id)
        .await
        .unwrap();
    assert!(matches!(
        only_draft
            .state
            .applications
            .set_standalone_settings(&only_draft.actor, &only_draft.app, Some(None), None)
            .await,
        Err(ServiceError::StandaloneProductConflict)
    ));
    assert!(sqlx::query("INSERT OR REPLACE INTO applications(id,slug,name,name_search) VALUES(?,'product','Product','product')")
        .bind(&only_draft.app).execute(&only_draft.state.database).await.is_err());
}

#[tokio::test]
async fn concurrent_settings_preserve_omitted_fields_and_serialize_first_signature() {
    let t = TestApp::unconfigured().await;
    let settings = format!("{}/integrations/standalone", t.base());
    let (identity, key) = tokio::join!(
        t.admin(
            "PUT",
            &settings,
            json!({"product_id":t.product}),
            Some(&t.token),
            true
        ),
        t.admin(
            "PUT",
            &settings,
            json!({"public_key":PUBLIC_KEY}),
            Some(&t.token),
            true
        )
    );
    assert_eq!(identity.status(), 204);
    assert_eq!(key.status(), 204);
    let app = t.state.applications.get(&t.app).await.unwrap();
    assert_eq!(app.product_id.as_deref(), Some(t.product.as_str()));
    assert_eq!(app.standalone_public_key.as_deref(), Some(PUBLIC_KEY));
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
    let shared = uuid::Uuid::new_v4().to_string();
    let (one, two) = tokio::join!(
        t.state
            .applications
            .set_standalone_settings(&t.actor, &t.app, Some(Some(&shared)), None),
        t.state.applications.set_standalone_settings(
            &t.actor,
            &other.id,
            Some(Some(&shared)),
            None
        )
    );
    assert!(matches!(
        (&one, &two),
        (Ok(()), Err(ServiceError::StandaloneProductConflict))
            | (Err(ServiceError::StandaloneProductConflict), Ok(()))
    ));
    t.state
        .applications
        .set_standalone_settings(&t.actor, &t.app, Some(Some(&t.product)), None)
        .await
        .unwrap();
    let r = t.create("0.5.0").await;
    let mut upload = t.begin(&r, "windows-x86_64").await;
    upload.write(CONTENT).await.unwrap();
    upload
        .set_standalone_signature(sign(&metadata(&t.product, "0.5.0", "windows-x86_64")).as_bytes())
        .unwrap();
    let next = uuid::Uuid::new_v4().to_string();
    let (signed, changed) = tokio::join!(
        t.state.artifacts.complete(&t.actor, upload),
        t.state
            .applications
            .set_standalone_settings(&t.actor, &t.app, Some(Some(&next)), None)
    );
    let app = t.state.applications.get(&t.app).await.unwrap();
    match (signed, changed) {
        (Ok(_), Err(ServiceError::StandaloneProductConflict)) => {
            assert_eq!(app.product_id.as_deref(), Some(t.product.as_str()));
            assert!(app.product_id_locked);
        }
        (Err(ServiceError::InvalidStandalone), Ok(())) => {
            assert_eq!(app.product_id.as_deref(), Some(next.as_str()));
            assert!(!app.product_id_locked);
            assert!(
                t.state
                    .artifacts
                    .list(&t.app, &r.id)
                    .await
                    .unwrap()
                    .is_empty()
            );
        }
        other => panic!("Unexpected identity/upload race: {other:?}"),
    }
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
                Some(&sign(&metadata(&t.product, "0.5.0+ci.7", target))),
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
        assert_eq!(signature, sign(&metadata(&t.product, version, target)));
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
    let valid = metadata(&t.product, "0.5.0", target);
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
        (&settings, json!({"product_id":t.product})),
        (
            &artifact,
            json!({"signed_metadata":sign(&metadata(&t.product,"0.5.0","windows-x86_64"))}),
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
            json!({"signed_metadata":sign(&metadata(&t.product,"0.5.0","windows-x86_64"))}),
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
    let jws = sign(&metadata(&t.product, "0.5.0", "windows-x86_64"));
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
            Some(&sign(&metadata(&t.product, "0.6.0", "windows-x86_64"))),
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
            Some(&sign(&metadata(&t.product, "0.6.0", "windows-x86_64"))),
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
            .set_standalone_signature(
                sign(&metadata(&t.product, "0.5.0", "linux-x86_64")).as_bytes(),
            )
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
        .arg(&t.product)
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
        .arg(&t.product)
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
            .arg(&t.product)
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
    let payload = metadata(&t.product, "0.5.0", "windows-x86_64");
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
