use crate::{
    domain::application::{Application, ApplicationPage, ListQuery, Sort, Status, Visibility},
    services::error::{Result, ServiceError},
};
use sqlx::{QueryBuilder, Sqlite, SqliteConnection, SqlitePool, Transaction};

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
        let row: StoredApplication = sqlx::query_as("SELECT * FROM applications WHERE id=?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or(ServiceError::NotFound)?;
        row.application()
    }
    pub async fn by_slug_in(c: &mut SqliteConnection, slug: &str) -> Result<Application> {
        sqlx::query_as::<_, StoredApplication>("SELECT * FROM applications WHERE slug=?")
            .bind(slug)
            .fetch_optional(c)
            .await?
            .ok_or(ServiceError::NotFound)?
            .application()
    }
    pub async fn get_in(c: &mut SqliteConnection, id: &str) -> Result<Application> {
        let row: StoredApplication = sqlx::query_as("SELECT * FROM applications WHERE id=?")
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
        let mut sql = filtered("SELECT * FROM applications", query);
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
    pub async fn update(c: &mut SqliteConnection, app: &Application) -> Result<()> {
        sqlx::query("UPDATE applications SET name=?,name_search=?,description=?,visibility=?,status=?,icon_key=?,updated_at=? WHERE id=?")
            .bind(&app.name).bind(app.name.to_lowercase()).bind(&app.description)
            .bind(app.visibility.as_str()).bind(app.status.as_str()).bind(&app.icon_key)
            .bind(app.updated_at).bind(&app.id).execute(c).await?;
        Ok(())
    }
    pub async fn icon_keys(&self) -> Result<Vec<String>> {
        Ok(
            sqlx::query_scalar("SELECT icon_key FROM applications WHERE icon_key IS NOT NULL")
                .fetch_all(&self.pool)
                .await?,
        )
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
