use std::{io::ErrorKind, process::Stdio, time::Duration};

use anyhow::{Context, Result, ensure};
use tokio::{net::TcpListener, process::Command, time::timeout};

use crate::config::AppConfig;

/// Keep the bound socket: a separate availability probe would race other processes.
pub async fn bind(config: &mut AppConfig) -> Result<TcpListener> {
    let requested = config.server.bind;
    let origin = format!("http://{requested}");
    // External origins (reverse proxy, Vite) require a stable upstream port.
    let local = requested.ip().is_loopback()
        && config.auth.public_origin == origin
        && config.download.public_base_url == origin;
    let listener = match TcpListener::bind(requested).await {
        Ok(listener) => listener,
        Err(error) if error.kind() == ErrorKind::AddrInUse && config.server.auto_port && local => {
            tracing::warn!(address = %requested, "HTTP port is occupied; selecting a free local port");
            TcpListener::bind((requested.ip(), 0))
                .await
                .context("Cannot bind a free local HTTP port")?
        }
        Err(error) => return Err(error).context("Cannot bind HTTP listener"),
    };
    let address = listener.local_addr().context("Cannot read HTTP address")?;
    config.server.bind = address;
    if local {
        let origin = format!("http://{address}");
        config.auth.public_origin = origin.clone();
        config.download.public_base_url = origin;
    }
    Ok(listener)
}

pub fn browser_url(config: &AppConfig) -> Option<String> {
    (config.server.open_browser
        && config.server.bind.ip().is_loopback()
        && config.auth.public_origin == format!("http://{}", config.server.bind))
    .then(|| format!("{}/admin", config.auth.public_origin))
}

/// Launch the OS browser without a shell; an unavailable desktop must not stop HTTP.
pub async fn open_browser(url: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    let mut command = Command::new("open");
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = Command::new("rundll32.exe");
        command.arg("url.dll,FileProtocolHandler");
        command
    };
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let mut command = Command::new("xdg-open");

    let status = timeout(
        Duration::from_secs(10),
        command
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .status(),
    )
    .await
    .context("Browser launcher timed out")?
    .context("Cannot start browser launcher")?;
    ensure!(status.success(), "Browser launcher failed: {status}");
    Ok(())
}
