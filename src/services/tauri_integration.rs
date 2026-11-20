use crate::{
    domain::{
        application::valid_slug,
        publication::{PublicationIssue, PublicationValidation},
    },
    repositories::{applications::ApplicationsRepository, distribution::PublicationSnapshot},
    services::{
        distribution::{PublicDistribution, public_application},
        error::{Result, ServiceError},
        publication_validation::PublicationValidator,
        updater,
    },
};
use serde_json::{Value, json};

pub fn endpoint(base: &str, slug: &str) -> Result<String> {
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
        "{}/api/v1/updates/tauri/{slug}/{{{{target}}}}/{{{{arch}}}}/{{{{current_version}}}}",
        base.trim_end_matches('/')
    ))
}
pub fn configuration(key: &str, endpoint: &str) -> Value {
    json!({"bundle":{"createUpdaterArtifacts":true},"plugins":{"updater":{"pubkey":key,"endpoints":[endpoint]}}})
}
pub struct UpdaterPackage {
    pub id: String,
    pub version: String,
    pub filename: String,
    pub target: String,
    pub compatible: bool,
    pub signature_present: bool,
}
pub struct IntegrationDetails {
    pub public_key: Option<String>,
    pub public_base_url: String,
    pub endpoint: String,
    pub configuration: Option<Value>,
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
    pub fn with_distribution(mut self, distribution: PublicDistribution) -> Self {
        self.distribution = distribution;
        self
    }
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
    pub async fn details(&self, id: &str, full_integrity: bool) -> Result<IntegrationDetails> {
        let catalog = self.distribution.published(id, false).await?;
        let app = &catalog.application;
        let (key, published) = self.applications.tauri_settings(id).await?;
        let endpoint = endpoint(&self.distribution.config.public_base_url, &app.slug)?;
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
        if public_application(app).is_err() {
            block(
                "endpoint_unavailable",
                "Приложение приватное или архивировано. Публичный updater недоступен.",
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
                "Для production Tauri требуется HTTPS; проверьте HTTPS и reverse proxy.",
            );
        }
        let mut packages = Vec::new();
        let mut warnings = Vec::new();
        let mut errors = Vec::new();
        for target in crate::domain::artifact::TARGETS {
            if crate::domain::publication::tauri_target(target).is_none() {
                continue;
            }
            if let updater::UpdateDecision::Available(s) =
                updater::select_latest(&catalog, target, "0.0.0-0")?
            {
                if packages
                    .iter()
                    .any(|p: &UpdaterPackage| p.id == s.artifact.id)
                {
                    continue;
                }
                packages.push(UpdaterPackage {
                    id: s.artifact.id.clone(),
                    version: s.release.version.to_string(),
                    filename: s.artifact.filename.clone(),
                    target: s.artifact.target.clone(),
                    compatible: true,
                    signature_present: true,
                });
                let snapshot = PublicationSnapshot {
                    application: app.clone(),
                    release: s.release.clone(),
                    artifacts: vec![s.artifact.clone()],
                };
                let v = if full_integrity {
                    self.validator.validate(&snapshot, false).await?
                } else {
                    self.validator.inspect(&snapshot).await?
                };
                warnings.extend(v.warnings);
                errors.extend(v.errors);
            }
        }
        if packages.is_empty() {
            block(
                "signed_updater_missing",
                "Нет совместимого опубликованного updater-пакета с корректной .sig.",
            );
        }
        for i in &errors {
            blockers.push(i.clone());
        }
        let validation =
            (!packages.is_empty()).then(|| PublicationValidation::new(warnings, errors));
        Ok(IntegrationDetails {
            configuration: key.as_deref().map(|key| configuration(key, &endpoint)),
            public_key: key,
            public_base_url: self.distribution.config.public_base_url.clone(),
            endpoint,
            published_release_exists: published,
            packages,
            blockers,
            validation,
        })
    }
}
