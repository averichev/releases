use crate::{
    domain::{
        application::Status,
        publication::{PublicationIssue, PublicationValidation, compatible_tauri_target},
        release::ReleaseStatus,
    },
    infrastructure::storage::{ArtifactStorage, LocalArtifactStorage},
    repositories::channel::PublicationSnapshot,
    services::error::{Result, ServiceError},
};
use std::collections::HashSet;
#[derive(Clone)]
pub struct PublicationValidator {
    storage: LocalArtifactStorage,
}
impl PublicationValidator {
    pub fn new(storage: LocalArtifactStorage) -> Self {
        Self { storage }
    }
    pub async fn validate(
        &self,
        snapshot: &PublicationSnapshot,
        require_draft: bool,
    ) -> Result<PublicationValidation> {
        self.check(snapshot, require_draft, true).await
    }
    /// Page inspection checks metadata without rehashing gigabytes. An explicit
    /// integrity check and publication still use validate() above.
    pub async fn inspect(&self, snapshot: &PublicationSnapshot) -> Result<PublicationValidation> {
        self.check(snapshot, false, false).await
    }
    async fn check(
        &self,
        snapshot: &PublicationSnapshot,
        require_draft: bool,
        full_integrity: bool,
    ) -> Result<PublicationValidation> {
        let mut warnings = Vec::new();
        let mut errors = Vec::new();
        let mut error = |code, message: &str| {
            errors.push(PublicationIssue {
                code,
                artifact_id: None,
                message: message.into(),
            })
        };
        if snapshot.application.status != Status::Active {
            error("application_archived", "Приложение находится в архиве.");
        }
        if require_draft && snapshot.release.status != ReleaseStatus::Draft {
            error(
                "release_published",
                "Релиз уже опубликован. Используйте переключение канала.",
            );
        }
        if snapshot.channel.name == "stable" && !snapshot.release.version.pre.is_empty() {
            error(
                "stable_prerelease",
                "Prerelease-версию нельзя назначить в stable.",
            );
        }
        if snapshot.artifacts.is_empty() {
            error("no_artifacts", "У релиза нет файлов.");
        }
        // Preference is read with the publication snapshot, before the write-time
        // revalidation performed by the publishing service.
        if let Some(selector) = &snapshot.default_distribution
            && !snapshot.artifacts.iter().any(|a| selector.matches(a))
        {
            warnings.push(PublicationIssue {
                code: "default_distribution_missing",
                artifact_id: None,
                message: "В релизе отсутствует настроенный дистрибутив по умолчанию.".into(),
            });
        }
        let mut targets = HashSet::new();
        let mut updater_targets = HashSet::new();
        for artifact in &snapshot.artifacts {
            let issue = |code, message: String| PublicationIssue {
                code,
                artifact_id: Some(artifact.id.clone()),
                message,
            };
            if !targets.insert((
                &artifact.target,
                &artifact.file_type,
                &artifact.distribution_role,
            )) {
                errors.push(issue(
                    "ambiguous_artifact",
                    format!(
                        "Несколько файлов для {} / {}.",
                        artifact.target, artifact.file_type
                    ),
                ));
            }
            let integrity = if full_integrity {
                self.storage
                    .verify(&artifact.storage_key, artifact.size_bytes, &artifact.sha256)
                    .await
            } else {
                self.storage
                    .open(&artifact.storage_key, artifact.size_bytes)
                    .await
                    .map(|_| true)
            };
            match integrity {
                Ok(true) => (),
                Ok(false) => errors.push(issue(
                    "checksum_mismatch",
                    format!(
                        "Контрольная сумма файла «{}» ({}) не совпадает.",
                        artifact.filename, artifact.target
                    ),
                )),
                Err(ServiceError::StorageUnavailable) => errors.push(issue(
                    "file_unavailable",
                    format!(
                        "Файл «{}» ({}) не найден или его размер не совпадает.",
                        artifact.filename, artifact.target
                    ),
                )),
                Err(other) => return Err(other),
            }
            if artifact.distribution_role == "updater" {
                if compatible_tauri_target(artifact).is_none() {
                    errors.push(issue(
                        "updater_incompatible",
                        format!(
                            "Формат {} несовместим с Tauri v2 для {}.",
                            artifact.file_type, artifact.target
                        ),
                    ));
                }
                if !updater_targets.insert(&artifact.target) {
                    errors.push(issue(
                        "ambiguous_updater",
                        format!(
                            "Для {} назначено несколько updater-пакетов. Оставьте один.",
                            artifact.target
                        ),
                    ));
                }
                if artifact.updater_signature.is_none() {
                    warnings.push(issue("signature_missing",format!("Отсутствует подпись Tauri Updater для {}. Этот файл будет недоступен в updater.",artifact.target)));
                }
            }
        }
        Ok(PublicationValidation::new(warnings, errors))
    }
}
