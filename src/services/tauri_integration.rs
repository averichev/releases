use crate::{
    domain::{
        application::valid_slug,
        publication::{PublicationIssue, PublicationValidation},
    },
    repositories::applications::ApplicationsRepository,
    services::{
        distribution::{self, PublicDistribution, public_application},
        error::{Result, ServiceError},
        publication_validation::PublicationValidator,
        updater,
    },
};
use serde_json::{Value, json};

/// One generator for the admin UI and any API consumer. Preserve Tauri tokens.
pub fn endpoint(base: &str, slug: &str, channel: &str) -> Result<String> {
    distribution::channel(channel)?;
    if !valid_slug(slug) {
        return Err(ServiceError::InvalidApplication);
    }
    let url = url::Url::parse(base).map_err(|_| ServiceError::InvalidUpdater)?;
    if !matches!(url.scheme(), "https" | "http")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return Err(ServiceError::InvalidUpdater);
    }
    Ok(format!(
        "{}/api/v1/updates/tauri/{slug}/{channel}/{{{{target}}}}/{{{{arch}}}}/{{{{current_version}}}}",
        base.trim_end_matches('/')
    ))
}
pub fn configuration(key: &str, endpoint: &str) -> Value {
    json!({"bundle":{"createUpdaterArtifacts":true},"plugins":{"updater":{"pubkey":key,"endpoints":[endpoint]}}})
}
pub struct UpdaterPackage {
    pub id: String,
    pub filename: String,
    pub target: String,
    pub compatible: bool,
    pub signature_present: bool,
}
pub struct IntegrationDetails {
    pub public_key: Option<String>,
    pub public_base_url: String,
    pub endpoint: String,
    pub endpoints: [String; 2],
    pub configuration: Option<Value>,
    pub channel: String,
    pub current_version: Option<String>,
    pub published_release_exists: bool,
    pub packages: Vec<UpdaterPackage>,
    pub blockers: Vec<PublicationIssue>,
    pub validation: Option<PublicationValidation>,
}
#[derive(Clone)]
pub struct TauriIntegration {
    applications: ApplicationsRepository,
    distribution: PublicDistribution,
    validator: PublicationValidator,
}
impl TauriIntegration {
    pub fn new(
        applications: ApplicationsRepository,
        distribution: PublicDistribution,
        validator: PublicationValidator,
    ) -> Self {
        Self {
            applications,
            distribution,
            validator,
        }
    }
    pub async fn details(
        &self,
        id: &str,
        channel: &str,
        full_integrity: bool,
    ) -> Result<IntegrationDetails> {
        let (app, channel, snapshot) = self.distribution.current(id, channel, false).await?;
        let (key, published) = self.applications.tauri_settings(id).await?;
        let endpoint = endpoint(
            &self.distribution.config.public_base_url,
            &app.slug,
            &channel.name,
        )?;
        let mut blockers = Vec::new();
        let mut block = |code, message: &str| {
            blockers.push(PublicationIssue {
                code,
                artifact_id: None,
                message: message.into(),
            })
        };
        if key.is_none() {
            block("public_key_missing", "Не настроен публичный ключ.");
        }
        if !published {
            block("published_release_missing", "Нет опубликованного релиза.");
        }
        if snapshot.is_none() {
            block("channel_release_missing", "Нет текущего релиза канала.");
        }
        if public_application(&app).is_err() {
            block(
                "endpoint_unavailable",
                "Серверный endpoint недоступен: приложение приватное или архивировано.",
            );
        }
        if !self
            .distribution
            .config
            .public_base_url
            .starts_with("https://")
        {
            block(
                "public_url_insecure",
                "Публичный адрес использует HTTP. Для production Tauri требуется HTTPS; проверьте download.public_base_url.",
            );
        }
        let mut packages = Vec::new();
        let validation = if let Some(s) = &snapshot {
            for a in s
                .artifacts
                .iter()
                .filter(|a| a.distribution_role == "updater")
            {
                packages.push(UpdaterPackage {
                    id: a.id.clone(),
                    filename: a.filename.clone(),
                    target: a.target.clone(),
                    compatible: crate::domain::publication::compatible_tauri_target(a).is_some(),
                    signature_present: a
                        .updater_signature
                        .as_deref()
                        .is_some_and(|v| updater::signature(v.as_bytes()).is_ok()),
                });
            }
            if packages.is_empty() {
                block("updater_package_missing", "Отсутствует updater-артефакт.");
            }
            if packages.iter().any(|a| !a.signature_present) {
                block(
                    "signature_missing",
                    "Отсутствует корректная .sig у одного или нескольких updater-пакетов.",
                );
            }
            let v = if full_integrity {
                self.validator.validate(s, false).await?
            } else {
                self.validator.inspect(s).await?
            };
            Some(v)
        } else {
            None
        };
        if let Some(v) = &validation {
            for i in &v.errors {
                blockers.push(PublicationIssue {
                    code: i.code,
                    artifact_id: i.artifact_id.clone(),
                    message: i.message.clone(),
                });
            }
        }
        Ok(IntegrationDetails {
            endpoints: [
                self::endpoint(
                    &self.distribution.config.public_base_url,
                    &app.slug,
                    "stable",
                )?,
                self::endpoint(&self.distribution.config.public_base_url, &app.slug, "beta")?,
            ],
            configuration: key.as_deref().map(|key| configuration(key, &endpoint)),
            public_key: key,
            public_base_url: self.distribution.config.public_base_url.clone(),
            endpoint,
            channel: channel.name,
            current_version: channel.current_version,
            published_release_exists: published,
            packages,
            blockers,
            validation,
        })
    }
}
