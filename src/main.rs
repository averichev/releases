use std::{future::IntoFuture, time::Duration};

use anyhow::{Context, Result};
use tokio::{net::TcpListener, sync::oneshot, time::timeout};
use veruna_releases::{
    config::AppConfig,
    http,
    infrastructure::{logging, shutdown::ShutdownSignal},
};

#[tokio::main]
async fn main() -> Result<()> {
    let config = AppConfig::load()?;
    logging::initialize(&config.logging.filter)?;
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if !arguments.is_empty() {
        let values: Vec<&str> = arguments.iter().map(String::as_str).collect();
        return match values.as_slice() {
            ["admin", "bootstrap"] => veruna_releases::cli::bootstrap(&config, false).await,
            ["admin", "bootstrap", "--stdin"] => {
                veruna_releases::cli::bootstrap(&config, true).await
            }
            ["admin", "reset-password"] => {
                veruna_releases::cli::reset_password(&config, false).await
            }
            ["admin", "reset-password", "--stdin"] => {
                veruna_releases::cli::reset_password(&config, true).await
            }
            _ => anyhow::bail!("Usage: veruna-releases [admin bootstrap|reset-password [--stdin]]"),
        };
    }

    let mut signal = ShutdownSignal::register().context("Cannot register shutdown signals")?;
    let database = veruna_releases::initialize(&config).await?;
    let state = veruna_releases::application_state(database.clone(), &config).await?;
    let listener = TcpListener::bind(config.server.bind)
        .await
        .context("Cannot bind HTTP listener")?;
    tracing::info!(address = %listener.local_addr()?, version = env!("CARGO_PKG_VERSION"), "Veruna Releases started");

    let grace = Duration::from_secs(config.server.shutdown_timeout_seconds);
    let (stop, stopped) = oneshot::channel::<()>();
    let server = axum::serve(listener, http::router(state))
        .with_graceful_shutdown(async {
            let _ = stopped.await;
        })
        .into_future();
    tokio::pin!(server);

    let result = tokio::select! {
        result = &mut server => result.context("HTTP server failed"),
        signal_result = signal.wait() => {
            tracing::info!("Shutdown requested; draining HTTP requests");
            let _ = stop.send(());
            match timeout(grace, &mut server).await {
                Ok(result) => result.context("HTTP server failed during shutdown")
                    .and(signal_result.context("Shutdown signal failed")),
                Err(_) => Err(anyhow::anyhow!("HTTP shutdown exceeded {} seconds", grace.as_secs())),
            }
        }
    };
    timeout(grace, database.close())
        .await
        .context("Database shutdown timed out")?;
    result?;
    tracing::info!("Veruna Releases stopped");
    Ok(())
}
