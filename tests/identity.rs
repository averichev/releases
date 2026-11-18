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
    config::AppConfig,
    http::{self, AppState},
    initialize,
    services::error::ServiceError,
};
const PASSWORD: &str = "strong-password-123";
const ORIGIN: &str = "http://127.0.0.1:8080";
struct TestApp {
    _dir: TempDir,
    state: AppState,
    router: Router,
}
struct Session {
    cookie: String,
    csrf: String,
    user: Value,
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
        let router = http::router(state.clone());
        Self {
            _dir: dir,
            state,
            router,
        }
    }
    async fn bootstrap(&self) {
        assert!(
            self.state
                .authentication
                .bootstrap("admin", PASSWORD.into())
                .await
                .unwrap()
        );
    }
    async fn send(
        &self,
        method: &str,
        path: &str,
        data: Value,
        session: Option<&Session>,
    ) -> Response {
        self.custom(method, path, data, session, Some(ORIGIN), true, true)
            .await
    }
    #[allow(clippy::too_many_arguments)]
    async fn custom(
        &self,
        method: &str,
        path: &str,
        data: Value,
        session: Option<&Session>,
        origin: Option<&str>,
        custom: bool,
        csrf: bool,
    ) -> Response {
        let mut builder = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json");
        if let Some(origin) = origin {
            builder = builder.header("origin", origin);
        }
        if custom {
            builder = builder.header("x-veruna-request", "1");
        }
        if let Some(session) = session {
            builder = builder.header("cookie", &session.cookie);
            if csrf {
                builder = builder.header("x-csrf-token", &session.csrf);
            }
        }
        self.router
            .clone()
            .oneshot(builder.body(Body::from(data.to_string())).unwrap())
            .await
            .unwrap()
    }
    async fn login(&self, username: &str, password: &str) -> Session {
        let response = self
            .send(
                "POST",
                "/api/v1/auth/login",
                json!({"username":username,"password":password}),
                None,
            )
            .await;
        assert_eq!(response.status(), StatusCode::OK);
        let cookie = response.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .to_owned();
        assert!(
            cookie.contains("HttpOnly")
                && cookie.contains("SameSite=Lax")
                && cookie.contains("Max-Age=43200")
        );
        let data = body(response).await;
        assert!(data["user"].get("password_hash").is_none());
        Session {
            cookie: cookie.split(';').next().unwrap().into(),
            csrf: data["csrf_token"].as_str().unwrap().into(),
            user: data["user"].clone(),
        }
    }
    async fn invite(&self, admin: &Session, username: &str, role: &str) -> String {
        let response = self
            .send(
                "POST",
                "/api/v1/admin/users/invitations",
                json!({"username":username,"role":role}),
                Some(admin),
            )
            .await;
        assert_eq!(response.status(), StatusCode::CREATED);
        body(response).await["activation_url"]
            .as_str()
            .unwrap()
            .split("#token=")
            .nth(1)
            .unwrap()
            .into()
    }
    async fn activate(&self, token: &str) -> Response {
        self.send(
            "POST",
            "/api/v1/auth/activate",
            json!({"token":token,"password":PASSWORD}),
            None,
        )
        .await
    }
}
async fn body(response: Response) -> Value {
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}
fn user_path(session: &Session, action: &str) -> String {
    format!(
        "/api/v1/admin/users/{}/{action}",
        session.user["id"].as_str().unwrap()
    )
}

#[tokio::test]
async fn bootstrap_is_idempotent_and_serialized() {
    let app = TestApp::new().await;
    let (a, b) = tokio::join!(
        app.state.authentication.bootstrap("first", PASSWORD.into()),
        app.state
            .authentication
            .bootstrap("second", PASSWORD.into())
    );
    assert_ne!(a.unwrap(), b.unwrap());
    assert!(
        !app.state
            .authentication
            .bootstrap("third", "invalid".into())
            .await
            .unwrap()
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(&app.state.database)
        .await
        .unwrap();
    assert_eq!(count, 1);
    let row: (String, String) = sqlx::query_as("SELECT password_hash,status FROM users")
        .fetch_one(&app.state.database)
        .await
        .unwrap();
    assert!(row.0.starts_with("$argon2id$"));
    assert_eq!(row.1, "active");
}
#[tokio::test]
async fn credentials_sessions_fixation_expiry_and_logout() {
    let app = TestApp::new().await;
    app.bootstrap().await;
    let bad = body(
        app.send(
            "POST",
            "/api/v1/auth/login",
            json!({"username":"admin","password":"wrong"}),
            None,
        )
        .await,
    )
    .await;
    let unknown = body(
        app.send(
            "POST",
            "/api/v1/auth/login",
            json!({"username":"nobody","password":"wrong"}),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(bad, unknown);
    assert_eq!(
        app.send(
            "POST",
            "/api/v1/auth/login",
            json!({"username":"admin' OR 1=1 --","password":PASSWORD}),
            None
        )
        .await
        .status(),
        401
    );
    let first = app.login("ADMIN", PASSWORD).await;
    let response = app
        .send(
            "POST",
            "/api/v1/auth/login",
            json!({"username":"admin","password":PASSWORD}),
            Some(&first),
        )
        .await;
    assert_eq!(response.status(), 200);
    assert_ne!(
        response.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap(),
        first.cookie
    );
    assert_eq!(
        app.send("GET", "/api/v1/auth/me", json!(null), Some(&first))
            .await
            .status(),
        401
    );
    let session = app.login("admin", PASSWORD).await;
    assert_eq!(
        app.send("GET", "/api/v1/auth/me", json!(null), Some(&session))
            .await
            .status(),
        200
    );
    let stored: Vec<String> = sqlx::query_scalar("SELECT token_hash FROM sessions")
        .fetch_all(&app.state.database)
        .await
        .unwrap();
    let raw = session.cookie.split('=').nth(1).unwrap();
    assert!(!stored.iter().any(|h| h == raw));
    assert_eq!(
        app.send("POST", "/api/v1/auth/logout", json!({}), Some(&session))
            .await
            .status(),
        204
    );
    assert_eq!(
        app.send("GET", "/api/v1/auth/me", json!(null), Some(&session))
            .await
            .status(),
        401
    );
    let session = app.login("admin", PASSWORD).await;
    // Make expiry deterministic, without sleeping.
    sqlx::query("UPDATE sessions SET expires_at=1, created_at=0")
        .execute(&app.state.database)
        .await
        .unwrap();
    assert_eq!(
        app.send("GET", "/api/v1/auth/me", json!(null), Some(&session))
            .await
            .status(),
        401
    );
}
#[tokio::test]
async fn invitations_permissions_roles_status_and_revoke() {
    let app = TestApp::new().await;
    app.bootstrap().await;
    let admin = app.login("admin", PASSWORD).await;
    let token = app.invite(&admin, "operator", "operator").await;
    assert_eq!(
        app.send(
            "POST",
            "/api/v1/auth/token/check",
            json!({"token":token}),
            None
        )
        .await
        .status(),
        200
    );
    assert_eq!(app.activate(&token).await.status(), 204);
    assert_eq!(app.activate(&token).await.status(), 400);
    let operator = app.login("operator", PASSWORD).await;
    for method in ["GET", "POST", "PATCH", "DELETE", "OPTIONS"] {
        assert_eq!(
            app.send(method, "/api/v1/admin/users", json!({"role":"admin"}), None)
                .await
                .status(),
            401
        );
        assert_eq!(
            app.send(
                method,
                "/api/v1/admin/users",
                json!({"role":"admin"}),
                Some(&operator)
            )
            .await
            .status(),
            403
        );
    }
    assert_eq!(
        app.send(
            "POST",
            &user_path(&operator, "revoke-sessions"),
            json!({}),
            Some(&admin)
        )
        .await
        .status(),
        204
    );
    assert_eq!(
        app.send("GET", "/api/v1/auth/me", json!(null), Some(&operator))
            .await
            .status(),
        401
    );
    let operator = app.login("operator", PASSWORD).await;
    assert_eq!(
        app.send(
            "PATCH",
            &user_path(&operator, "role"),
            json!({"role":"root"}),
            Some(&admin)
        )
        .await
        .status(),
        400
    );
    assert_eq!(
        app.send(
            "PATCH",
            &user_path(&operator, "status"),
            json!({"status":"disabled"}),
            Some(&admin)
        )
        .await
        .status(),
        204
    );
    assert_eq!(
        app.send("GET", "/api/v1/auth/me", json!(null), Some(&operator))
            .await
            .status(),
        401
    );
    assert_eq!(
        app.send(
            "POST",
            "/api/v1/auth/login",
            json!({"username":"operator","password":PASSWORD}),
            None
        )
        .await
        .status(),
        401
    );
    assert_eq!(
        app.send(
            "PATCH",
            &user_path(&operator, "status"),
            json!({"status":"active"}),
            Some(&admin)
        )
        .await
        .status(),
        204
    );
    let operator = app.login("operator", PASSWORD).await;
    assert_eq!(
        app.send(
            "PATCH",
            &user_path(&operator, "role"),
            json!({"role":"admin"}),
            Some(&admin)
        )
        .await
        .status(),
        204
    );
    let promoted = app.login("operator", PASSWORD).await;
    assert_eq!(
        app.send("GET", "/api/v1/admin/users", json!(null), Some(&promoted))
            .await
            .status(),
        200
    );
}
#[tokio::test]
async fn token_expiry_reissue_and_administrative_password_recovery() {
    let app = TestApp::new().await;
    app.bootstrap().await;
    let admin = app.login("admin", PASSWORD).await;
    let old = app.invite(&admin, "invitee", "operator").await;
    let id: String = sqlx::query_scalar("SELECT id FROM users WHERE username='invitee'")
        .fetch_one(&app.state.database)
        .await
        .unwrap();
    let response = app
        .send(
            "POST",
            &format!("/api/v1/admin/users/{id}/invitation"),
            json!({}),
            Some(&admin),
        )
        .await;
    assert_eq!(response.status(), 200);
    let new = body(response).await["activation_url"]
        .as_str()
        .unwrap()
        .split("#token=")
        .nth(1)
        .unwrap()
        .to_owned();
    assert_eq!(app.activate(&old).await.status(), 400);
    sqlx::query("UPDATE user_tokens SET expires_at=0")
        .execute(&app.state.database)
        .await
        .unwrap();
    assert_eq!(app.activate(&new).await.status(), 400);
    let response = app
        .send(
            "POST",
            &format!("/api/v1/admin/users/{id}/invitation"),
            json!({}),
            Some(&admin),
        )
        .await;
    let token = body(response).await["activation_url"]
        .as_str()
        .unwrap()
        .split("#token=")
        .nth(1)
        .unwrap()
        .to_owned();
    // Concurrent consumption must succeed exactly once.
    let (a, b) = tokio::join!(app.activate(&token), app.activate(&token));
    assert!((a.status() == 204 && b.status() == 400) || (a.status() == 400 && b.status() == 204));
    let operator = app.login("invitee", PASSWORD).await;
    let response = app
        .send(
            "POST",
            &user_path(&operator, "password-reset"),
            json!({}),
            Some(&admin),
        )
        .await;
    let token = body(response).await["activation_url"]
        .as_str()
        .unwrap()
        .split("#token=")
        .nth(1)
        .unwrap()
        .to_owned();
    assert_eq!(app.activate(&token).await.status(), 204);
    assert_eq!(
        app.send("GET", "/api/v1/auth/me", json!(null), Some(&operator))
            .await
            .status(),
        401
    );
    assert_eq!(app.activate(&token).await.status(), 400);
}
#[tokio::test]
async fn own_password_change_revokes_all_sessions_and_outstanding_tokens() {
    let app = TestApp::new().await;
    app.bootstrap().await;
    let first = app.login("admin", PASSWORD).await;
    let second = app.login("admin", PASSWORD).await;
    let response = app
        .send(
            "POST",
            &user_path(&first, "password-reset"),
            json!({}),
            Some(&first),
        )
        .await;
    let token = body(response).await["activation_url"]
        .as_str()
        .unwrap()
        .split("#token=")
        .nth(1)
        .unwrap()
        .to_owned();
    assert_eq!(
        app.send(
            "POST",
            "/api/v1/auth/change-password",
            json!({"current_password":"wrong","new_password":"new-password-456"}),
            Some(&first)
        )
        .await
        .status(),
        401
    );
    assert_eq!(
        app.send(
            "POST",
            "/api/v1/auth/change-password",
            json!({"current_password":PASSWORD,"new_password":"short"}),
            Some(&first)
        )
        .await
        .status(),
        400
    );
    assert_eq!(
        app.send(
            "POST",
            "/api/v1/auth/change-password",
            json!({"current_password":PASSWORD,"new_password":"new-password-456"}),
            Some(&first)
        )
        .await
        .status(),
        204
    );
    for session in [&first, &second] {
        assert_eq!(
            app.send("GET", "/api/v1/auth/me", json!(null), Some(session))
                .await
                .status(),
            401
        );
    }
    assert_eq!(app.activate(&token).await.status(), 400);
    assert_eq!(
        app.send(
            "POST",
            "/api/v1/auth/login",
            json!({"username":"admin","password":PASSWORD}),
            None
        )
        .await
        .status(),
        401
    );
    app.login("admin", "new-password-456").await;
}
#[tokio::test]
async fn last_administrator_and_concurrent_demotion_are_protected() {
    let app = TestApp::new().await;
    app.bootstrap().await;
    let admin = app.login("admin", PASSWORD).await;
    for (action, payload) in [
        ("role", json!({"role":"operator"})),
        ("status", json!({"status":"disabled"})),
    ] {
        let response = app
            .send("PATCH", &user_path(&admin, action), payload, Some(&admin))
            .await;
        assert_eq!(response.status(), 409);
        assert_eq!(body(response).await["error"]["code"], "last_active_admin");
    }
    let response = app
        .send(
            "POST",
            "/api/v1/admin/users",
            json!({"username":"second","role":"admin","password":PASSWORD}),
            Some(&admin),
        )
        .await;
    assert_eq!(response.status(), 201);
    let second = app.login("second", PASSWORD).await;
    let admin_path = user_path(&admin, "role");
    let second_path = user_path(&second, "role");
    let (a, b) = tokio::join!(
        app.send(
            "PATCH",
            &admin_path,
            json!({"role":"operator"}),
            Some(&admin)
        ),
        app.send(
            "PATCH",
            &second_path,
            json!({"role":"operator"}),
            Some(&second)
        )
    );
    assert!((a.status() == 204 && b.status() == 409) || (a.status() == 409 && b.status() == 204));
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM users WHERE role='admin' AND status='active'")
            .fetch_one(&app.state.database)
            .await
            .unwrap();
    assert_eq!(count, 1);
}
#[tokio::test]
async fn csrf_json_limits_and_login_bruteforce() {
    let app = TestApp::new().await;
    app.bootstrap().await;
    let admin = app.login("admin", PASSWORD).await;
    for (origin, custom, csrf) in [
        (None, true, true),
        (Some("https://attacker.invalid"), true, true),
        (Some(ORIGIN), false, true),
        (Some(ORIGIN), true, false),
    ] {
        assert_eq!(
            app.custom(
                "POST",
                "/api/v1/auth/logout",
                json!({}),
                Some(&admin),
                origin,
                custom,
                csrf
            )
            .await
            .status(),
            403
        );
    }
    assert_eq!(
        app.custom(
            "POST",
            "/api/v1/auth/login",
            json!({"username":"admin","password":PASSWORD}),
            None,
            Some("https://evil.invalid"),
            true,
            true
        )
        .await
        .status(),
        403
    );
    assert_eq!(
        app.send(
            "POST",
            "/api/v1/auth/login",
            json!({"username":"admin","password":PASSWORD,"role":"admin"}),
            None
        )
        .await
        .status(),
        400
    );
    let response = app
        .send(
            "POST",
            "/api/v1/auth/login",
            json!({"username":"admin","password":"x".repeat(9000)}),
            None,
        )
        .await;
    assert_eq!(response.status(), 400);
    for _ in 0..5 {
        assert_eq!(
            app.send(
                "POST",
                "/api/v1/auth/login",
                json!({"username":"admin","password":"wrong"}),
                None
            )
            .await
            .status(),
            401
        );
    }
    assert_eq!(
        app.send(
            "POST",
            "/api/v1/auth/login",
            json!({"username":"ADMIN","password":PASSWORD}),
            None
        )
        .await
        .status(),
        429
    );
    sqlx::query("UPDATE login_attempts SET expires_at=0")
        .execute(&app.state.database)
        .await
        .unwrap();
    app.login("admin", PASSWORD).await;
}
#[tokio::test]
async fn stale_administrative_identity_cannot_mutate_after_revoke() {
    let app = TestApp::new().await;
    app.bootstrap().await;
    let admin = app.login("admin", PASSWORD).await;
    let raw = admin.cookie.split('=').nth(1).unwrap();
    let identity = app.state.sessions.authenticate(raw).await.unwrap();
    app.state.sessions.logout(&identity).await.unwrap();
    assert!(matches!(
        app.state.users.revoke(&identity, &identity.user.id).await,
        Err(ServiceError::Unauthorized)
    ));
}

#[tokio::test]
async fn https_cookie_policy_ignores_forwarded_headers() {
    let mut app = TestApp::new().await;
    app.bootstrap().await;
    app.state.auth_config.public_origin = "https://releases.example.com".into();
    app.router = http::router(app.state.clone());
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header("origin", "https://releases.example.com")
        .header("x-veruna-request", "1")
        .header("content-type", "application/json")
        .header("x-forwarded-proto", "http")
        .header("forwarded", "proto=http;for=attacker")
        .body(Body::from(
            json!({"username":"admin","password":PASSWORD}).to_string(),
        ))
        .unwrap();
    let response = app.router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), 200);
    let cookie = response.headers()["set-cookie"].to_str().unwrap();
    assert!(cookie.starts_with("__Host-veruna_session="));
    assert!(cookie.contains("; Secure"));
    assert!(!cookie.contains("Domain="));
    app.state.auth_config.public_origin = ORIGIN.into();
    app.router = http::router(app.state.clone());
    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/login")
                .header("origin", ORIGIN)
                .header("x-veruna-request", "1")
                .header("content-type", "application/json")
                .header("x-forwarded-proto", "https")
                .body(Body::from(
                    json!({"username":"admin","password":PASSWORD}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let cookie = response.headers()["set-cookie"].to_str().unwrap();
    assert!(cookie.starts_with("veruna_session="));
    assert!(!cookie.contains("; Secure"));
}
#[tokio::test]
async fn blocking_an_invitation_invalidates_it_and_unblocking_requires_reissue() {
    let app = TestApp::new().await;
    app.bootstrap().await;
    let admin = app.login("admin", PASSWORD).await;
    let token = app.invite(&admin, "waiting", "operator").await;
    let id: String = sqlx::query_scalar("SELECT id FROM users WHERE username='waiting'")
        .fetch_one(&app.state.database)
        .await
        .unwrap();
    let path = format!("/api/v1/admin/users/{id}/status");
    assert_eq!(
        app.send("PATCH", &path, json!({"status":"disabled"}), Some(&admin))
            .await
            .status(),
        204
    );
    assert_eq!(app.activate(&token).await.status(), 400);
    assert_eq!(
        app.send("PATCH", &path, json!({"status":"active"}), Some(&admin))
            .await
            .status(),
        204
    );
    let response = app
        .send(
            "POST",
            &format!("/api/v1/admin/users/{id}/invitation"),
            json!({}),
            Some(&admin),
        )
        .await;
    assert_eq!(response.status(), 200);
    let token = body(response).await["activation_url"]
        .as_str()
        .unwrap()
        .split("#token=")
        .nth(1)
        .unwrap()
        .to_owned();
    assert_eq!(app.activate(&token).await.status(), 204);
}

#[tokio::test]
async fn automatic_origin_uses_each_proxy_request_and_keeps_csrf_and_secure_cookies() {
    use axum::extract::ConnectInfo;
    let mut app = TestApp::new().await;
    app.bootstrap().await;
    app.state.auth_config.public_origin.clear();
    app.state.distribution.config.public_base_url.clear();
    app.router = http::router(app.state.clone());
    for (index, host) in ["releases.example.test", "another.example.test:8443"]
        .into_iter()
        .enumerate()
    {
        let origin = format!("https://{host}");
        let response = app
            .router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/auth/login")
                    .extension(ConnectInfo(
                        "127.0.0.1:12345".parse::<std::net::SocketAddr>().unwrap(),
                    ))
                    .header("host", host)
                    .header("x-forwarded-proto", "https")
                    .header("x-forwarded-host", "attacker.example")
                    .header("forwarded", "host=attacker.example;proto=http")
                    .header("origin", &origin)
                    .header("x-veruna-request", "1")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({"username":"admin","password":PASSWORD}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let cookie = response.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .to_string();
        assert!(cookie.starts_with("__Host-veruna_session="));
        assert!(cookie.contains("; Secure") && !cookie.contains("Domain="));
        let cookie = cookie.split(';').next().unwrap();
        let csrf = body(response).await["csrf_token"]
            .as_str()
            .unwrap()
            .to_owned();
        for (request_origin, expected) in [("https://attacker.example", 403), (&origin, 201)] {
            let response = app
                .router
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/v1/admin/users/invitations")
                        .extension(ConnectInfo(
                            "127.0.0.1:12345".parse::<std::net::SocketAddr>().unwrap(),
                        ))
                        .header("host", host)
                        .header("x-forwarded-proto", "https")
                        .header("origin", request_origin)
                        .header("x-veruna-request", "1")
                        .header("cookie", cookie)
                        .header("x-csrf-token", &csrf)
                        .header("content-type", "application/json")
                        .body(Body::from(
                            json!({"username":format!("invited-{index}"),"role":"operator"})
                                .to_string(),
                        ))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), expected);
            if expected == 201 {
                assert!(
                    body(response).await["activation_url"]
                        .as_str()
                        .unwrap()
                        .starts_with(&format!("{origin}/admin/activate#token="))
                );
            }
        }
    }
    assert!(app.state.auth_config.public_origin.is_empty());
    assert!(app.state.distribution.config.public_base_url.is_empty());
}

#[tokio::test]
async fn automatic_origin_rejects_rebinding_untrusted_proxy_and_ambiguous_headers() {
    use axum::extract::ConnectInfo;
    let mut app = TestApp::new().await;
    app.state.auth_config.public_origin.clear();
    app.state.distribution.config.public_base_url.clear();
    app.router = http::router(app.state.clone());
    for (peer, host, proto) in [
        ("203.0.113.10:12345", "releases.example.test", Some("https")),
        ("127.0.0.1:12345", "attacker.example", None),
        (
            "127.0.0.1:12345",
            "releases.example.test",
            Some("https,http"),
        ),
        (
            "127.0.0.1:12345",
            "user@releases.example.test",
            Some("https"),
        ),
        (
            "127.0.0.1:12345",
            "releases.example.test/path",
            Some("https"),
        ),
        ("127.0.0.1:12345", "releases.example.test:0", Some("https")),
    ] {
        let mut request = Request::builder()
            .uri("/api/v1/health")
            .extension(ConnectInfo(peer.parse::<std::net::SocketAddr>().unwrap()))
            .header("host", host);
        if let Some(proto) = proto {
            request = request.header("x-forwarded-proto", proto);
        }
        assert_eq!(
            app.router
                .clone()
                .oneshot(request.body(Body::empty()).unwrap())
                .await
                .unwrap()
                .status(),
            400,
            "{peer} {host} {proto:?}"
        );
    }
    let request = Request::builder()
        .uri("/api/v1/health")
        .extension(ConnectInfo(
            "127.0.0.1:12345".parse::<std::net::SocketAddr>().unwrap(),
        ))
        .header("host", "releases.example.test")
        .header("host", "attacker.example")
        .header("x-forwarded-proto", "https")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app.router.clone().oneshot(request).await.unwrap().status(),
        400
    );
}
