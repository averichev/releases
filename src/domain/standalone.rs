//! Bounded JWS profile for standalone executables. Signing happens off-server.
use super::{artifact::Artifact, release::Release};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};

pub const MAX_SIGNATURE_BYTES: usize = 4096;
pub const MEDIA_TYPE: &str = "releases-standalone+jws";

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Metadata {
    pub schema_version: u32,
    pub application_id: String,
    pub version: String,
    pub os: String,
    pub arch: String,
    pub size_bytes: u64,
    pub sha256: String,
}

/// Only concrete, canonical targets. No automatic architecture/OS fallback.
pub fn target(value: &str) -> bool {
    super::artifact::TARGETS.contains(&value) && !matches!(value, "any" | "darwin-universal")
}
pub fn compatible(artifact: &Artifact) -> bool {
    target(&artifact.target)
        && artifact.distribution_role == "installer"
        && if artifact.target.starts_with("windows-") {
            artifact.file_type == "exe"
        } else {
            artifact.file_type == "bin"
        }
}
/// Public Ed25519 JWK `x`, exactly 32 bytes; private-key packets are rejected.
pub fn public_key(value: &str) -> bool {
    if value.len() != 43 {
        return false;
    }
    URL_SAFE_NO_PAD
        .decode(value)
        .ok()
        .and_then(|b| <[u8; 32]>::try_from(b).ok())
        .and_then(|b| ed25519_dalek::VerifyingKey::from_bytes(&b).ok())
        .is_some_and(|key| !key.is_weak())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProtectedHeader {
    alg: String,
    typ: String,
}

pub fn verify(
    jws: &str,
    key: &str,
    application_id: &str,
    release: &Release,
    artifact: &Artifact,
) -> Option<Metadata> {
    if jws.is_empty()
        || jws.len() > MAX_SIGNATURE_BYTES
        || !public_key(key)
        || !compatible(artifact)
        || artifact.release_id != release.id
        || release.application_id != application_id
    {
        return None;
    }
    // Narrow profile rejects embedded/remote keys, critical extensions, detached
    // payloads and algorithm negotiation. The JOSE library verifies the signature.
    let header = URL_SAFE_NO_PAD.decode(jws.split('.').next()?).ok()?;
    let header: ProtectedHeader = serde_json::from_slice(&header).ok()?;
    if header.alg != "EdDSA" || header.typ != MEDIA_TYPE {
        return None;
    }
    let mut validation = Validation::new(Algorithm::EdDSA);
    validation.required_spec_claims.clear();
    validation.validate_exp = false; // Immutable release metadata, not a login JWT.
    let decoded = jsonwebtoken::decode::<Metadata>(
        jws,
        &DecodingKey::from_ed_components(key).ok()?,
        &validation,
    )
    .ok()?;
    let m = decoded.claims;
    let (os, arch) = artifact.target.split_once('-')?;
    (m.schema_version == 1
        && m.application_id == application_id
        && m.version == release.version.to_string()
        && m.os == os
        && m.arch == arch
        && artifact.size_bytes > 0
        && m.size_bytes == artifact.size_bytes as u64
        && m.sha256 == artifact.sha256)
        .then_some(m)
}
