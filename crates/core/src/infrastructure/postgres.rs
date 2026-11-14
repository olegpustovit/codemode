#[derive(Clone)]
pub struct Database(pub(crate) sqlx::PgPool, i64);

const MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

#[derive(Debug, thiserror::Error)]
#[error("database operation failed")]
pub struct Error(#[source] pub(crate) sqlx::Error);

impl From<sqlx::Error> for Error {
    fn from(error: sqlx::Error) -> Self {
        Self(error)
    }
}

impl Database {
    pub async fn connect(url: &str) -> Result<Self, Error> {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(16)
            .acquire_timeout(std::time::Duration::from_secs(5))
            .connect(url)
            .await?;
        let mut migrator = MIGRATOR;
        // Newer additive migrations must not prevent a binary rollback.
        migrator.set_ignore_missing(true);
        migrator
            .run(&pool)
            .await
            .map_err(|e| Error(sqlx::Error::Migrate(Box::new(e))))?;
        let version=sqlx::query_scalar!("SELECT COALESCE(max(version),0)::bigint AS \"version!\" FROM _sqlx_migrations WHERE success").fetch_one(&pool).await?;
        Ok(Self(pool, version))
    }

    pub async fn for_tenant(&self, tenant: uuid::Uuid) -> Result<Self, Error> {
        let options = (*self.0.connect_options()).clone();
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(4)
            .idle_timeout(std::time::Duration::from_secs(30))
            .acquire_timeout(std::time::Duration::from_secs(5))
            .after_connect(move |connection, _| {
                Box::pin(async move {
                    sqlx::query("SET ROLE codemode_tenant")
                        .execute(&mut *connection)
                        .await?;
                    sqlx::query("SELECT set_config('codemode.tenant_id',$1,false)")
                        .bind(tenant.to_string())
                        .execute(&mut *connection)
                        .await?;
                    Ok(())
                })
            })
            .connect_with(options)
            .await?;
        Ok(Self(pool, self.1))
    }

    pub async fn healthy(&self) -> bool {
        sqlx::query_scalar::<_, i32>("SELECT 1")
            .fetch_one(&self.0)
            .await
            .is_ok()
    }

    pub fn schema_version(&self) -> i64 {
        self.1
    }

    pub async fn close(&self) {
        self.0.close().await;
    }
}
