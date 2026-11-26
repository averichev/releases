use std::sync::Arc;

use standalone_updater::{OperationStatus, UpdateError, UpdateMonitor};

use crate::{
    config::SelfUpdateMode,
    domain::user::Role,
    infrastructure::secrets,
    repositories::sessions::SessionsRepository,
    services::{
        error::{Result, ServiceError},
        sessions::{Identity, Sessions},
    },
};

#[derive(Clone)]
pub enum ServiceUpdates {
    Disabled,
    Monitor(Arc<UpdateMonitor>),
    #[cfg(target_os = "linux")]
    Linux(standalone_updater::runtime::UpdateManager),
}

pub struct Summary {
    pub mode: Option<SelfUpdateMode>,
    pub installation_supported: bool,
    pub installation_reason: Option<String>,
    pub status: Option<OperationStatus>,
}

pub enum Action {
    Check,
    Prepare { check_id: String, version: String },
    Install { operation_id: String },
    Cancel { operation_id: String },
}

impl ServiceUpdates {
    pub async fn summary(&self) -> Result<Summary> {
        match self {
            Self::Disabled => Ok(Summary {
                mode: None,
                installation_supported: false,
                installation_reason: None,
                status: None,
            }),
            Self::Monitor(monitor) => Ok(Summary {
                mode: Some(SelfUpdateMode::External),
                installation_supported: false,
                installation_reason: Some("Обновления устанавливаются вручную.".into()),
                status: Some(monitor.status()?),
            }),
            #[cfg(target_os = "linux")]
            Self::Linux(manager) => {
                use standalone_updater::runtime::InstallationMode;
                let manager = manager.clone();
                let summary = tokio::task::spawn_blocking(move || manager.summary())
                    .await
                    .map_err(anyhow::Error::from)??;
                Ok(Summary {
                    mode: Some(match summary.installation_mode {
                        InstallationMode::Supervisor => SelfUpdateMode::Supervisor,
                        InstallationMode::Standalone => SelfUpdateMode::Standalone,
                        InstallationMode::External => SelfUpdateMode::External,
                    }),
                    installation_supported: summary.installation_supported,
                    installation_reason: summary.installation_reason,
                    status: Some(summary.status),
                })
            }
        }
    }

    pub async fn execute(
        &self,
        sessions: &Sessions,
        actor: &Identity,
        action: Action,
    ) -> Result<OperationStatus> {
        let mut tx = sessions.repository.write().await?;
        let user =
            SessionsRepository::authorize(&mut tx, &actor.session_hash, secrets::now()).await?;
        if user.role != Role::Admin {
            return Err(ServiceError::Forbidden);
        }
        let updates = self.clone();
        // Hold the authorization lock through dispatch, including if the HTTP caller
        // disconnects. Downloads and installation run under the library's own locks.
        tokio::task::spawn_blocking(move || {
            tokio::runtime::Handle::current().block_on(async move {
                let status = updates.dispatch(action).await?;
                tx.commit().await?;
                Ok(status)
            })
        })
        .await
        .map_err(anyhow::Error::from)?
    }

    async fn dispatch(&self, action: Action) -> standalone_updater::Result<OperationStatus> {
        match (self, action) {
            (Self::Monitor(monitor), Action::Check) => monitor.check(),
            (Self::Disabled, _) => Err(UpdateError::Unsupported(
                "источник обновлений сервиса не настроен",
            )),
            (Self::Monitor(_), _) => Err(UpdateError::Unsupported(
                "включена только проверка обновлений",
            )),
            #[cfg(target_os = "linux")]
            (Self::Linux(manager), action) => match action {
                Action::Check => manager.check(),
                Action::Prepare { check_id, version } => manager
                    .prepare(standalone_updater::runtime::InstallRequest { check_id, version }),
                Action::Install { operation_id } => manager.install(&operation_id).await,
                Action::Cancel { operation_id } => manager.cancel_prepared(&operation_id),
            },
        }
    }

    pub async fn shutdown_requested(&self) {
        #[cfg(target_os = "linux")]
        if let Self::Linux(manager) = self {
            return manager.shutdown_requested().await;
        }
        std::future::pending().await
    }

    pub fn finish_shutdown(&self) -> Result<()> {
        #[cfg(target_os = "linux")]
        if let Self::Linux(manager) = self {
            manager.finish_shutdown()?;
        }
        Ok(())
    }
}
