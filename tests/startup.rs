use std::io::ErrorKind;

use axum::{body::Body, http::Request};
use tempfile::tempdir;
use tokio::net::TcpListener;
use tower::ServiceExt;
use veruna_releases::{config::AppConfig, infrastructure::startup};

fn local_config(address: std::net::SocketAddr) -> AppConfig {
    let mut config = AppConfig::default();
    config.server.bind = address;
    config.auth.public_origin = format!("http://{address}");
    config.download.public_base_url = config.auth.public_origin.clone();
    config
}

#[tokio::test]
async fn occupied_local_port_is_reserved_and_origins_follow_actual_listener() {
    let occupied = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let requested = occupied.local_addr().unwrap();
    let mut config = local_config(requested);
    let listener = startup::bind(&mut config).await.unwrap();
    let address = listener.local_addr().unwrap();
    assert_ne!(address.port(), requested.port());
    assert_eq!(address.ip(), requested.ip());
    assert_eq!(config.server.bind, address);
    assert_eq!(config.auth.public_origin, format!("http://{address}"));
    assert_eq!(config.download.public_base_url, config.auth.public_origin);
    assert_eq!(
        startup::browser_url(&config),
        Some(format!("http://{address}/admin"))
    );
    assert_eq!(
        TcpListener::bind(address).await.unwrap_err().kind(),
        ErrorKind::AddrInUse
    );
    config.validate().unwrap();
}

#[tokio::test]
async fn fixed_ports_and_external_origins_do_not_silently_move() {
    let occupied = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = occupied.local_addr().unwrap();
    for scenario in ["disabled", "proxy", "vite", "download"] {
        let mut config = local_config(address);
        match scenario {
            "disabled" => config.server.auto_port = false,
            "proxy" => config.auth.public_origin = "https://releases.example.com".into(),
            "vite" => config.auth.public_origin = "http://127.0.0.1:5173".into(),
            "download" => config.download.public_base_url = "https://cdn.example.com".into(),
            _ => unreachable!(),
        }
        let auth = config.auth.public_origin.clone();
        let download = config.download.public_base_url.clone();
        assert!(startup::bind(&mut config).await.is_err(), "{scenario}");
        assert_eq!(config.server.bind, address);
        assert_eq!(config.auth.public_origin, auth);
        assert_eq!(config.download.public_base_url, download);
    }
    let wildcard = TcpListener::bind("0.0.0.0:0").await.unwrap();
    let mut config = local_config(wildcard.local_addr().unwrap());
    assert!(startup::bind(&mut config).await.is_err());
    assert!(startup::browser_url(&config).is_none());
}

#[tokio::test]
async fn explicit_dynamic_port_and_browser_opt_out_work() {
    let mut config = local_config("127.0.0.1:0".parse().unwrap());
    let listener = startup::bind(&mut config).await.unwrap();
    assert_ne!(listener.local_addr().unwrap().port(), 0);
    assert!(startup::browser_url(&config).is_some());
    config.server.open_browser = false;
    assert!(startup::browser_url(&config).is_none());
    config.server.open_browser = true;
    config.auth.public_origin = "https://releases.example.com".into();
    assert!(startup::browser_url(&config).is_none());
}

#[tokio::test]
async fn fallback_keeps_csrf_strict_on_the_selected_origin() {
    let occupied = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut config = local_config(occupied.local_addr().unwrap());
    let old_origin = config.auth.public_origin.clone();
    let directory = tempdir().unwrap();
    config.database.path = directory.path().join("db.sqlite");
    config.storage.directory = directory.path().join("artifacts");
    config.storage.icons_directory = directory.path().join("icons");
    let database = veruna_releases::initialize(&config).await.unwrap();
    let _listener = startup::bind(&mut config).await.unwrap();
    let state = veruna_releases::application_state(database.clone(), &config)
        .await
        .unwrap();
    assert_eq!(state.auth_config.public_origin, config.auth.public_origin);
    let app = veruna_releases::http::router(state);
    for (origin, expected) in [(&old_origin, 403), (&config.auth.public_origin, 401)] {
        let response = app
            .clone()
            .oneshot(
                Request::post("/api/v1/auth/login")
                    .header("content-type", "application/json")
                    .header("origin", origin)
                    .header("x-veruna-request", "1")
                    .body(Body::from(
                        r#"{"username":"missing","password":"secret-password-123"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
    }
    database.close().await;
}

#[cfg(unix)]
mod desktop {
    use std::{
        fs::{self, File, Permissions},
        os::unix::fs::PermissionsExt,
        process::{Child, Command, Stdio},
        time::Duration,
    };

    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
        time::{sleep, timeout},
    };

    struct Server(Child);
    impl Drop for Server {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    #[tokio::test]
    async fn executable_opens_selected_port_once_and_survives_launcher_failure() {
        for launcher_exit in [0, 1] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let launcher = root.join(if cfg!(target_os = "macos") {
                "open"
            } else {
                "xdg-open"
            });
            fs::write(
                &launcher,
                format!("#!/bin/sh\nprintf '%s\\n' \"$1\" >> browser-url\nexit {launcher_exit}\n"),
            )
            .unwrap();
            fs::set_permissions(&launcher, Permissions::from_mode(0o755)).unwrap();
            let occupied = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = occupied.local_addr().unwrap();
            let origin = format!("http://{address}");
            let log = File::create(root.join("server.log")).unwrap();
            let mut server = Server(
                Command::new(env!("CARGO_BIN_EXE_veruna-releases"))
                    .env_clear()
                    .env("PATH", root)
                    .env("SERVER__BIND", address.to_string())
                    .env("AUTH__PUBLIC_ORIGIN", &origin)
                    .env("DOWNLOAD__PUBLIC_BASE_URL", &origin)
                    .current_dir(root)
                    .stdin(Stdio::null())
                    .stdout(log.try_clone().unwrap())
                    .stderr(log)
                    .spawn()
                    .unwrap(),
            );
            let url = timeout(Duration::from_secs(15), async {
                loop {
                    let log = fs::read_to_string(root.join("server.log")).unwrap();
                    assert!(server.0.try_wait().unwrap().is_none(), "{log}");
                    if let Ok(url) = fs::read_to_string(root.join("browser-url"))
                        && (launcher_exit == 0 || log.contains("Cannot open browser automatically"))
                    {
                        break url;
                    }
                    sleep(Duration::from_millis(25)).await;
                }
            })
            .await
            .expect("Browser launcher was not called");
            assert_eq!(url.lines().count(), 1, "Browser must open once");
            let url = url::Url::parse(url.trim()).unwrap();
            assert_eq!(url.path(), "/admin");
            assert_eq!(url.host_str(), Some("127.0.0.1"));
            assert_ne!(url.port().unwrap(), address.port());
            let mut stream = TcpStream::connect(("127.0.0.1", url.port().unwrap()))
                .await
                .unwrap();
            stream
                .write_all(
                    b"GET /api/v1/health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
                )
                .await
                .unwrap();
            let mut response = String::new();
            timeout(Duration::from_secs(5), stream.read_to_string(&mut response))
                .await
                .unwrap()
                .unwrap();
            assert!(response.starts_with("HTTP/1.1 200"), "{response}");
            assert!(response.contains(r#""status":"ok""#), "{response}");
        }
    }
}

#[tokio::test]
async fn automatic_origins_stay_automatic_after_port_fallback() {
    let occupied = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut config = AppConfig::default();
    config.server.bind = occupied.local_addr().unwrap();
    let listener = startup::bind(&mut config).await.unwrap();
    assert_ne!(config.server.bind, occupied.local_addr().unwrap());
    assert_eq!(config.server.bind, listener.local_addr().unwrap());
    assert!(config.auth.public_origin.is_empty());
    assert!(config.download.public_base_url.is_empty());
    assert_eq!(
        startup::browser_url(&config),
        Some(format!("http://{}/admin", config.server.bind))
    );
}
