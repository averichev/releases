use crate::{
    domain::user::valid_password,
    services::error::{Result, ServiceError},
};
use argon2::{
    Algorithm, Argon2, Params, PasswordHasher, PasswordVerifier, Version,
    password_hash::{PasswordHash, SaltString},
};
use std::sync::Arc;
use tokio::sync::Semaphore;
use zeroize::Zeroizing;

#[derive(Clone)]
pub struct Passwords {
    slots: Arc<Semaphore>,
    dummy: Arc<String>,
}
impl Passwords {
    pub async fn new() -> Result<Self> {
        let dummy =
            tokio::task::spawn_blocking(|| hash_sync(Zeroizing::new(super::secrets::random()?)))
                .await
                .map_err(|e| ServiceError::Internal(e.into()))??;
        Ok(Self {
            slots: Arc::new(Semaphore::new(4)),
            dummy: Arc::new(dummy),
        })
    }
    pub async fn hash(&self, password: String) -> Result<String> {
        let password = Zeroizing::new(password);
        if !valid_password(&password) {
            return Err(ServiceError::Invalid);
        }
        let permit = self
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| ServiceError::RateLimited)?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            hash_sync(password)
        })
        .await
        .map_err(|e| ServiceError::Internal(e.into()))?
    }
    pub async fn verify(&self, password: String, hash: Option<String>) -> Result<bool> {
        let password = Zeroizing::new(password);
        let permit = self
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| ServiceError::RateLimited)?;
        let hash = hash.unwrap_or_else(|| self.dummy.as_ref().clone());
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let parsed = PasswordHash::new(&hash).map_err(|_| {
                ServiceError::Internal(anyhow::anyhow!("Invalid stored password hash"))
            })?;
            Ok(argon()
                .verify_password(password.as_bytes(), &parsed)
                .is_ok())
        })
        .await
        .map_err(|e| ServiceError::Internal(e.into()))?
    }
}
fn argon() -> Argon2<'static> {
    Argon2::new(Algorithm::Argon2id, Version::V0x13, Params::default())
}
fn hash_sync(password: Zeroizing<String>) -> Result<String> {
    let mut salt = [0u8; 16];
    getrandom::fill(&mut salt)
        .map_err(|_| ServiceError::Internal(anyhow::anyhow!("OS randomness unavailable")))?;
    let salt = SaltString::encode_b64(&salt)
        .map_err(|_| ServiceError::Internal(anyhow::anyhow!("Cannot encode salt")))?;
    argon()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|_| ServiceError::Internal(anyhow::anyhow!("Cannot hash password")))
}
