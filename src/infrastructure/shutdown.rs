use std::io;

/// Register both Unix signals before starting to accept requests.
#[cfg(unix)]
pub struct ShutdownSignal {
    interrupt: tokio::signal::unix::Signal,
    terminate: tokio::signal::unix::Signal,
}

#[cfg(unix)]
impl ShutdownSignal {
    pub fn register() -> io::Result<Self> {
        use tokio::signal::unix::{SignalKind, signal};
        Ok(Self {
            interrupt: signal(SignalKind::interrupt())?,
            terminate: signal(SignalKind::terminate())?,
        })
    }

    pub async fn wait(&mut self) -> io::Result<()> {
        tokio::select! {
            _ = self.interrupt.recv() => {},
            _ = self.terminate.recv() => {},
        }
        Ok(())
    }
}

#[cfg(not(unix))]
pub struct ShutdownSignal;

#[cfg(not(unix))]
impl ShutdownSignal {
    pub fn register() -> io::Result<Self> {
        Ok(Self)
    }
    pub async fn wait(&mut self) -> io::Result<()> {
        tokio::signal::ctrl_c().await
    }
}
