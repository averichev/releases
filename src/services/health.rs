use sqlx::SqlitePool;

use crate::repositories;

#[derive(Debug, thiserror::Error)]
#[error("database unavailable")]
pub struct HealthError(#[source] pub sqlx::Error);

pub async fn check(pool: &SqlitePool) -> Result<(), HealthError> {
    repositories::health::ping(pool).await.map_err(HealthError)
}
