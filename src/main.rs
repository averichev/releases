use std::{future::IntoFuture, time::Duration};

use anyhow::{Context, Result};
use tokio::{sync::oneshot, time::timeout};
use veruna_releases::{
    config::AppConfig,
    http,
    infrastructure::{logging, shutdown::ShutdownSignal, startup},
};

#[tokio::main]
async fn main() -> Result<()> {
    let mut config = AppConfig::load()?;
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
    let listener = startup::bind(&mut config).await?;
    let state = veruna_releases::application_state(database.clone(), &config).await?;
    tracing::info!(address = %listener.local_addr()?, version = env!("CARGO_PKG_VERSION"), "Veruna Releases started");

    let grace = Duration::from_secs(config.server.shutdown_timeout_seconds);
    let (stop, stopped) = oneshot::channel::<()>();
    let server = axum::serve(listener, http::router(state))
        .with_graceful_shutdown(async {
            let _ = stopped.await;
        })
        .into_future();
    tokio::pin!(server);

    // Poll the server first so it is accepting requests before the browser opens.
    let browser = async {
        if let Some(url) = startup::browser_url(&config) {
            tracing::info!(url = %url, "Open the administration panel in your browser");
            if let Err(error) = startup::open_browser(&url).await {
                tracing::warn!(%error, "Cannot open browser automatically; use the administration URL above");
            }
        }
    };
    tokio::pin!(browser);
    let mut browser_done = false;
    let result = loop {
        tokio::select! {
            biased;
            result = &mut server => break result.context("HTTP server failed"),
            signal_result = signal.wait() => {
                tracing::info!("Shutdown requested; draining HTTP requests");
                let _ = stop.send(());
                break match timeout(grace, &mut server).await {
                    Ok(result) => result.context("HTTP server failed during shutdown")
                        .and(signal_result.context("Shutdown signal failed")),
                    Err(_) => Err(anyhow::anyhow!("HTTP shutdown exceeded {} seconds", grace.as_secs())),
                };
            }
            () = &mut browser, if !browser_done => browser_done = true,
        }
    };
    timeout(grace, database.close())
        .await
        .context("Database shutdown timed out")?;
    result?;
    tracing::info!("Veruna Releases stopped");
    Ok(())
}
