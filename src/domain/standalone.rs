//! Bounded JWS profile for standalone executables. Signing happens off-server.
use super::{application::Application, artifact::Artifact, release::Release};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use jsonwebtoken::jwk::{Jwk, ThumbprintHash};
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};

pub const MAX_SIGNATURE_BYTES: usize = 4096;
pub const MEDIA_TYPE: &str = "releases-standalone+jws";

#[derive(Debug, PartialEq, Eq)]
pub enum MetadataField {
    SchemaVersion,
    ProductId,
    Version,
    Os,
    Arch,
    SizeBytes,
    Sha256,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum VerificationError {
    #[error("Standalone product identity and public key are not configured")]
    ConfigurationMissing,
    #[error("Standalone product identity is not configured")]
    ProductIdMissing,
    #[error("Standalone public key is not configured")]
    PublicKeyMissing,
    #[error("Invalid standalone public key")]
    InvalidPublicKey,
    #[error("Artifact is incompatible with standalone updates")]
    IncompatibleArtifact,
    #[error("Standalone artifact ownership mismatch")]
    OwnershipMismatch,
    #[error("Invalid compact JWS file")]
    InvalidJws,
    #[error("Invalid protected JWS header")]
    InvalidHeader,
    #[error("JWS algorithm must be EdDSA")]
    InvalidAlgorithm,
    #[error("Invalid JWS media type")]
    InvalidMediaType,
    #[error("JWS key identifier does not match the trusted public key")]
    KeyIdMismatch,
    #[error("JWS signature verification failed")]
    InvalidSignature,
    #[error("Invalid signed metadata schema")]
    InvalidMetadata,
    // Display deliberately omits claim values; callers must not log request contents.
    #[error("Signed standalone metadata does not match")]
    MetadataMismatch {
        field: MetadataField,
        expected: String,
        actual: String,
    },
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Metadata {
    pub schema_version: u32,
    /// Publisher's product_id; the v1 wire name is retained for compatibility.
    pub application_id: String,
    pub version: String,
    pub os: String,
    pub arch: String,
    pub size_bytes: u64,
    pub sha256: String,
}

/// Only concrete, canonical targets. No automatic architecture/OS fallback.
pub fn target(value: &str) -> bool {
    super::artifact::TARGETS.contains(&value)
        && matches!(value.split_once('-'), Some((_, "x86_64" | "aarch64")))
}
pub fn compatible(artifact: &Artifact) -> bool {
    target(&artifact.target)
        && match artifact.distribution_role.as_str() {
            // Executables may have no extension, version dots, .tool or .x86_64.
            // The signed os/arch identifies the ABI; a filename suffix does not.
            "executable" => true,
            "installer" if artifact.target.starts_with("windows-") => artifact.file_type == "exe",
            "installer" => artifact.file_type == "bin",
            _ => false,
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

/// RFC 7638 identifier, derived from a locally trusted Ed25519 key.
/// It selects a key; it never establishes trust or fetches a remote key.
pub fn key_id(value: &str) -> Option<String> {
    if !public_key(value) {
        return None;
    }
    Jwk::from_decoding_key(
        &DecodingKey::from_ed_components(value).ok()?,
        Some(Algorithm::EdDSA),
    )
    .ok()?
    .thumbprint(ThumbprintHash::SHA256)
    .ok()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProtectedHeader {
    alg: String,
    typ: String,
    #[serde(default, deserialize_with = "deserialize_key_id")]
    kid: Option<String>,
}

fn deserialize_key_id<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    // Missing is compatible with the original profile; explicit null is not a JWS kid.
    String::deserialize(deserializer).map(Some)
}

pub fn product_id(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok_and(|id| id.to_string() == value)
}

/// One source of identity/key selection, with local ownership checked separately.
pub fn verify(
    jws: &str,
    application: &Application,
    release: &Release,
    artifact: &Artifact,
) -> Result<Metadata, VerificationError> {
    use VerificationError as E;
    let (product_id, key) = match (
        application.product_id.as_deref(),
        application.standalone_public_key.as_deref(),
    ) {
        (None, None) => return Err(E::ConfigurationMissing),
        (None, _) => return Err(E::ProductIdMissing),
        (_, None) => return Err(E::PublicKeyMissing),
        (Some(product_id), Some(key)) => (product_id, key),
    };
    if !public_key(key) {
        return Err(E::InvalidPublicKey);
    }
    if artifact.release_id != release.id || release.application_id != application.id {
        return Err(E::OwnershipMismatch);
    }
    if !compatible(artifact) || artifact.size_bytes <= 0 {
        return Err(E::IncompatibleArtifact);
    }
    if jws.is_empty() || jws.len() > MAX_SIGNATURE_BYTES {
        return Err(E::InvalidJws);
    }
    // Narrow profile rejects embedded/remote keys, critical extensions, detached
    // payloads and algorithm negotiation. The JOSE library verifies the signature.
    let header = URL_SAFE_NO_PAD
        .decode(jws.split('.').next().ok_or(E::InvalidJws)?)
        .map_err(|_| E::InvalidJws)?;
    let header: ProtectedHeader = serde_json::from_slice(&header).map_err(|_| E::InvalidHeader)?;
    if header.alg != "EdDSA" {
        return Err(E::InvalidAlgorithm);
    }
    if header.typ != MEDIA_TYPE {
        return Err(E::InvalidMediaType);
    }
    if header
        .kid
        .as_deref()
        .is_some_and(|kid| key_id(key).as_deref() != Some(kid))
    {
        return Err(E::KeyIdMismatch);
    }
    let mut validation = Validation::new(Algorithm::EdDSA);
    validation.required_spec_claims.clear();
    validation.validate_exp = false; // Immutable release metadata, not a login JWT.
    let decoded = jsonwebtoken::decode::<Metadata>(
        jws,
        &DecodingKey::from_ed_components(key).map_err(|_| E::InvalidPublicKey)?,
        &validation,
    )
    .map_err(|error| match error.kind() {
        jsonwebtoken::errors::ErrorKind::InvalidSignature => E::InvalidSignature,
        jsonwebtoken::errors::ErrorKind::Json(_) => E::InvalidMetadata,
        _ => E::InvalidJws,
    })?;
    let m = decoded.claims;
    let (os, arch) = artifact
        .target
        .split_once('-')
        .ok_or(E::IncompatibleArtifact)?;
    // Claims are compared and exposed only after cryptographic verification.
    let compare = |field, expected: &str, actual: &str| {
        if expected == actual {
            Ok(())
        } else {
            Err(E::MetadataMismatch {
                field,
                expected: expected.to_owned(),
                actual: actual.to_owned(),
            })
        }
    };
    compare(
        MetadataField::SchemaVersion,
        "1",
        &m.schema_version.to_string(),
    )?;
    compare(MetadataField::ProductId, product_id, &m.application_id)?;
    compare(
        MetadataField::Version,
        &release.version.to_string(),
        &m.version,
    )?;
    compare(MetadataField::Os, os, &m.os)?;
    compare(MetadataField::Arch, arch, &m.arch)?;
    compare(
        MetadataField::SizeBytes,
        &artifact.size_bytes.to_string(),
        &m.size_bytes.to_string(),
    )?;
    compare(MetadataField::Sha256, &artifact.sha256, &m.sha256)?;
    Ok(m)
}
