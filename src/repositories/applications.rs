use crate::{
    domain::application::{Application, ApplicationPage, ListQuery, Sort, Status, Visibility},
    services::error::{Result, ServiceError},
};
use sqlx::{QueryBuilder, Sqlite, SqliteConnection, SqlitePool, Transaction};

const SELECT: &str = "SELECT applications.*, (SELECT coalesce(sum(d.download_count),0) FROM releases r JOIN artifacts a ON a.release_id=r.id JOIN artifact_download_counts d ON d.artifact_id=a.id WHERE r.application_id=applications.id) AS download_count FROM applications";

#[derive(sqlx::FromRow)]
struct StoredApplication {
    id: String,
    slug: String,
    name: String,
    description: String,
    icon_key: Option<String>,
    visibility: String,
    status: String,
    created_at: i64,
    updated_at: i64,
    standalone_public_key: Option<String>,
    product_id: Option<String>,
    product_id_locked: bool,
    download_count: i64,
}
impl StoredApplication {
    fn application(self) -> Result<Application> {
        Ok(Application {
            id: self.id,
            slug: self.slug,
            name: self.name,
            description: self.description,
            icon_key: self.icon_key,
            visibility: Visibility::parse(&self.visibility).ok_or_else(|| {
                ServiceError::Internal(anyhow::anyhow!("Invalid stored application visibility"))
            })?,
            status: Status::parse(&self.status).ok_or_else(|| {
                ServiceError::Internal(anyhow::anyhow!("Invalid stored application status"))
            })?,
            created_at: self.created_at,
            updated_at: self.updated_at,
            standalone_public_key: self.standalone_public_key,
            product_id: self.product_id,
            product_id_locked: self.product_id_locked,
            download_count: self.download_count,
        })
    }
}
#[derive(Clone)]
pub struct ApplicationsRepository {
    pool: SqlitePool,
}
impl ApplicationsRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
    pub async fn write(&self) -> Result<Transaction<'static, Sqlite>> {
        Ok(self.pool.begin_with("BEGIN IMMEDIATE").await?)
    }
    pub async fn get(&self, id: &str) -> Result<Application> {
        let row: StoredApplication = sqlx::query_as(&format!("{SELECT} WHERE id=?"))
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or(ServiceError::NotFound)?;
        row.application()
    }
    pub async fn standalone_settings(&self, id: &str) -> Result<(Application, bool)> {
        let mut tx = self.pool.begin().await?;
        let app = Self::get_in(&mut tx, id).await?;
        let key_locked = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM artifacts a JOIN releases r ON r.id=a.release_id WHERE r.application_id=? AND a.standalone_signature IS NOT NULL)",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok((app, key_locked))
    }
    pub async fn by_slug_in(c: &mut SqliteConnection, slug: &str) -> Result<Application> {
        sqlx::query_as::<_, StoredApplication>(&format!("{SELECT} WHERE slug=?"))
            .bind(slug)
            .fetch_optional(c)
            .await?
            .ok_or(ServiceError::NotFound)?
            .application()
    }
    pub async fn get_in(c: &mut SqliteConnection, id: &str) -> Result<Application> {
        let row: StoredApplication = sqlx::query_as(&format!("{SELECT} WHERE id=?"))
            .bind(id)
            .fetch_optional(c)
            .await?
            .ok_or(ServiceError::NotFound)?;
        row.application()
    }
    pub async fn list(&self, query: &ListQuery) -> Result<ApplicationPage> {
        // Count and page share a read snapshot, including during concurrent changes.
        let mut tx = self.pool.begin().await?;
        let total: i64 = filtered("SELECT count(*) FROM applications", query)
            .build_query_scalar()
            .fetch_one(&mut *tx)
            .await?;
        let mut sql = filtered(SELECT, query);
        sql.push(" ORDER BY ").push(match query.sort {
            Sort::Name => "name_search",
            Sort::CreatedAt => "created_at",
            Sort::UpdatedAt => "updated_at",
        });
        sql.push(if query.descending { " DESC" } else { " ASC" });
        sql.push(", id ASC LIMIT ")
            .push_bind(i64::from(query.per_page))
            .push(" OFFSET ")
            .push_bind(i64::from(query.page - 1) * i64::from(query.per_page));
        let rows: Vec<StoredApplication> = sql.build_query_as().fetch_all(&mut *tx).await?;
        tx.commit().await?;
        Ok(ApplicationPage {
            items: rows
                .into_iter()
                .map(StoredApplication::application)
                .collect::<Result<_>>()?,
            total,
        })
    }
    pub async fn insert(c: &mut SqliteConnection, app: &Application) -> Result<()> {
        let result = sqlx::query("INSERT INTO applications(id,slug,name,name_search,description,visibility,status,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?)")
            .bind(&app.id).bind(&app.slug).bind(&app.name).bind(app.name.to_lowercase())
            .bind(&app.description).bind(app.visibility.as_str()).bind(app.status.as_str())
            .bind(app.created_at).bind(app.updated_at).execute(c).await;
        match result {
            Err(sqlx::Error::Database(ref db)) if db.is_unique_violation() => {
                Err(ServiceError::ApplicationSlugConflict)
            }
            other => {
                other?;
                Ok(())
            }
        }
    }
    pub async fn public_page_in(
        c: &mut SqliteConnection,
        limit: u32,
        offset: u32,
    ) -> Result<ApplicationPage> {
        const FILTER: &str = " WHERE status='active' AND visibility='public' AND EXISTS (SELECT 1 FROM releases WHERE application_id=applications.id AND status='published')";
        let total = sqlx::query_scalar(&format!("SELECT count(*) FROM applications{FILTER}"))
            .fetch_one(&mut *c)
            .await?;
        let rows: Vec<StoredApplication> = sqlx::query_as(&format!(
            "{SELECT}{FILTER} ORDER BY name_search, slug LIMIT ? OFFSET ?"
        ))
        .bind(i64::from(limit))
        .bind(i64::from(offset))
        .fetch_all(c)
        .await?;
        Ok(ApplicationPage {
            items: rows
                .into_iter()
                .map(StoredApplication::application)
                .collect::<Result<_>>()?,
            total,
        })
    }
    pub async fn update(c: &mut SqliteConnection, app: &Application) -> Result<()> {
        sqlx::query("UPDATE applications SET name=?,name_search=?,description=?,visibility=?,status=?,icon_key=?,updated_at=? WHERE id=?")
            .bind(&app.name).bind(app.name.to_lowercase()).bind(&app.description)
            .bind(app.visibility.as_str()).bind(app.status.as_str()).bind(&app.icon_key)
            .bind(app.updated_at).bind(&app.id).execute(c).await?;
        Ok(())
    }
    pub async fn tauri_settings(&self, id: &str) -> Result<(Option<String>, bool)> {
        sqlx::query_as("SELECT tauri_public_key, EXISTS(SELECT 1 FROM releases WHERE application_id=applications.id AND status='published') FROM applications WHERE id=?")
            .bind(id).fetch_optional(&self.pool).await?.ok_or(ServiceError::NotFound)
    }
    pub async fn set_tauri_key(
        c: &mut SqliteConnection,
        id: &str,
        key: Option<&str>,
        now: i64,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE applications SET tauri_public_key=?,updated_at=max(updated_at,?) WHERE id=?",
        )
        .bind(key)
        .bind(now)
        .bind(id)
        .execute(c)
        .await?;
        Ok(())
    }
    pub async fn icon_keys(&self) -> Result<Vec<String>> {
        Ok(
            sqlx::query_scalar("SELECT icon_key FROM applications WHERE icon_key IS NOT NULL")
                .fetch_all(&self.pool)
                .await?,
        )
    }
    pub async fn set_standalone_settings(
        c: &mut SqliteConnection,
        id: &str,
        key: Option<&str>,
        product_id: Option<&str>,
        now: i64,
    ) -> Result<()> {
        let result = sqlx::query("UPDATE applications SET standalone_public_key=?,product_id=?,updated_at=max(updated_at,?) WHERE id=?")
            .bind(key).bind(product_id).bind(now).bind(id).execute(c).await;
        match result {
            Err(sqlx::Error::Database(ref e))
                if e.message() == "standalone_product_locked"
                    || (e.is_unique_violation()
                        && e.message().contains("applications.product_id")) =>
            {
                Err(ServiceError::StandaloneProductConflict)
            }
            Err(sqlx::Error::Database(ref e))
                if e.is_unique_violation() || e.message() == "standalone_key_locked" =>
            {
                Err(ServiceError::StandaloneKeyConflict)
            }
            other => {
                other?;
                Ok(())
            }
        }
    }
}
fn filtered<'a>(select: &'static str, query: &'a ListQuery) -> QueryBuilder<'a, Sqlite> {
    let mut sql = QueryBuilder::new(select);
    sql.push(" WHERE 1=1");
    if let Some(status) = query.status {
        sql.push(" AND status=").push_bind(status.as_str());
    }
    if let Some(visibility) = query.visibility {
        sql.push(" AND visibility=").push_bind(visibility.as_str());
    }
    if !query.search.is_empty() {
        // instr treats %, _ and SQL metacharacters literally.
        sql.push(" AND (instr(name_search,")
            .push_bind(query.search.to_lowercase())
            .push(")>0 OR instr(slug,")
            .push_bind(query.search.to_lowercase())
            .push(")>0)");
    }
    sql
}
