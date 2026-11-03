use sqlx::SqlitePool;

pub async fn ping(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT 1").execute(pool).await?;
    Ok(())
}
