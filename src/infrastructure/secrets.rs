use crate::services::error::{Result, ServiceError};
use sha2::{Digest, Sha256};
pub fn random() -> Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes)
        .map_err(|_| ServiceError::Internal(anyhow::anyhow!("OS randomness unavailable")))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
pub fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}
pub fn valid(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
pub fn csrf(value: &str) -> String {
    digest(&format!("veruna-csrf:{value}"))
}
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}
