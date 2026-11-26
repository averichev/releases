//! Process dispatch runs before opening business data or binding the HTTP listener.
use anyhow::{Result, ensure};
use standalone_updater::{PublicKey, UpdateMonitor, UpdateSource};

use crate::{
    config::{AppConfig, SelfUpdateMode},
    services::self_update::ServiceUpdates,
};

pub const PRODUCT_ID: &str = env!("APP_PRODUCT_ID");

pub fn target() -> String {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        os => os,
    };
    format!("{os}-{}", std::env::consts::ARCH)
}

fn source(manifest_url: &url::Url) -> UpdateSource {
    UpdateSource::from_manifest(
        manifest_url.as_str(),
        PRODUCT_ID,
        PublicKey {
            kty: "OKP".into(),
            crv: "Ed25519".into(),
            x: env!("APP_UPDATE_PUBLIC_KEY").into(),
        },
        vec![manifest_url.origin().ascii_serialization()],
        256 * 1024 * 1024,
    )
}

pub async fn launch(config: &AppConfig, arguments: &[String]) -> Result<ServiceUpdates> {
    let settings = &config.self_update;
    let Some(manifest_url) = settings.parsed_manifest_url()? else {
        ensure!(
            arguments.is_empty(),
            "Updater role requires a configured Linux installation"
        );
        return Ok(ServiceUpdates::Disabled);
    };
    let source = source(&manifest_url);
    if config.self_update_mode() == SelfUpdateMode::External {
        ensure!(
            arguments.is_empty(),
            "Updater role unavailable in external mode"
        );
        let monitor = UpdateMonitor::new(source, crate::VERSION, &target())?;
        return Ok(ServiceUpdates::Monitor(monitor.into()));
    }
    #[cfg(target_os = "linux")]
    {
        use standalone_updater::runtime::{self, Launch};
        let runtime = linux::runtime_config(config, source)?;
        linux::check_directory_separation(config, &runtime.updates_directory)?;
        match runtime::launch(runtime).await? {
            Launch::Exit(code) => std::process::exit(code),
            Launch::Serve(manager) => Ok(ServiceUpdates::Linux(manager)),
        }
    }
    #[cfg(not(target_os = "linux"))]
    anyhow::bail!("Self-installation requires Linux")
}

#[cfg(target_os = "linux")]
pub mod linux {
    use super::*;
    use anyhow::Context;
    use standalone_updater::runtime::{
        self, BuildInfo, DataPolicy, InstallationMode, Readiness, RuntimeConfig,
    };
    use std::{
        io::ErrorKind,
        path::{Component, Path, PathBuf},
    };

    pub const READINESS_PATH: &str = "/api/v1/health/self-update";

    pub fn build_info() -> BuildInfo {
        BuildInfo {
            version: crate::VERSION.into(),
            product_id: PRODUCT_ID.into(),
            rust_target: env!("APP_BUILD_TARGET").into(),
            self_update_protocol: 1,
            data_compatibility: crate::infrastructure::database::data_compatibility(),
        }
    }

    pub fn internal_role(arguments: &[String]) -> bool {
        matches!(arguments, [role] if matches!(role.as_str(), runtime::SERVER_ARGUMENT | runtime::WORKER_ARGUMENT | runtime::RECOVER_ARGUMENT))
    }

    // Business directories may still be missing on first startup. Resolve existing
    // ancestors (including symlinks) before comparing the intended locations.
    fn resolved(path: &Path) -> Result<PathBuf> {
        let mut result = PathBuf::new();
        for component in std::path::absolute(path)?.components() {
            if component == Component::ParentDir {
                result.pop();
                continue;
            }
            result.push(component);
            match result.symlink_metadata() {
                Ok(_) => result = result.canonicalize()?,
                Err(error)
                    if matches!(error.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(result)
    }

    pub(super) fn check_directory_separation(config: &AppConfig, directory: &Path) -> Result<()> {
        let updates = resolved(directory)?;
        for path in [
            &config.database.path,
            &config.storage.directory,
            &config.storage.icons_directory,
            &config.storage.temporary_directory(),
        ] {
            let path = resolved(path)?;
            ensure!(
                !path.starts_with(&updates) && !updates.starts_with(&path),
                "self_update.directory must be separate from release storage and database"
            );
        }
        Ok(())
    }

    pub(super) fn runtime_config(
        config: &AppConfig,
        source: UpdateSource,
    ) -> Result<RuntimeConfig> {
        Ok(RuntimeConfig {
            source,
            build: build_info(),
            mode: match config.self_update_mode() {
                SelfUpdateMode::Supervisor => InstallationMode::Supervisor,
                SelfUpdateMode::Standalone => InstallationMode::Standalone,
                SelfUpdateMode::External => InstallationMode::External,
            },
            executable: std::env::current_exe()?,
            updates_directory: config.self_update_directory(),
            // The server currently accepts no application arguments.
            arguments: Vec::new(),
            working_directory: std::env::current_dir()?,
            data_policy: DataPolicy::RequireExact,
            readiness: Readiness::http(config.server.bind, READINESS_PATH),
        })
    }

    pub fn dispatch_admin(config: &AppConfig, arguments: &[String]) -> Result<()> {
        use std::os::unix::process::CommandExt;
        let settings = &config.self_update;
        if config.self_update_mode() == SelfUpdateMode::Supervisor
            && let Some(manifest_url) = settings.parsed_manifest_url()?
        {
            let runtime = runtime_config(config, source(&manifest_url))?;
            // The first admin bootstrap needs only SQLite, even if artifact storage is unavailable.
            if let Some(active) = runtime::active_executable(&runtime)?
                && active != runtime.executable
            {
                check_directory_separation(config, &runtime.updates_directory)?;
                return Err(std::process::Command::new(active).args(arguments).exec())
                    .context("Cannot start the active release for this admin command");
            }
        }
        Ok(())
    }
}
