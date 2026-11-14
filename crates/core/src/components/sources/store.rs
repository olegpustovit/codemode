use super::*;
use secrecy::SecretString;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    pub id: Uuid,
    #[serde(flatten)]
    pub source: Source,
    pub status: String,
    pub last_error: Option<String>,
    pub synced_at: Option<String>,
    pub tool_count: i64,
    pub revision: i64,
    pub python_name: String,
}

impl Sources {
    pub async fn list(&self) -> Result<Vec<Record>, Error> {
        self.ensure_names().await?;
        let rows = sqlx::query!(
            r#"SELECT s.id,s.config AS "config: sqlx::types::Json<Source>",s.status,s.last_error,
            to_char(s.synced_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS synced_at,
            (SELECT count(*) FROM tools t WHERE t.source_id=s.id) AS "tool_count!",s.revision,s.python_name AS "python_name!"
            FROM sources s ORDER BY s.slug LIMIT 64"#
        )
        .fetch_all(&self.db.0)
        .await
        .map_err(DbError::from)?;
        Ok(rows
            .into_iter()
            .map(|row| Record {
                id: row.id,
                source: row.config.0,
                status: row.status,
                last_error: row.last_error,
                synced_at: row.synced_at,
                tool_count: row.tool_count,
                revision: row.revision,
                python_name: row.python_name,
            })
            .collect())
    }

    pub async fn get(&self, id: Uuid) -> Result<Option<Record>, Error> {
        Ok(self
            .list()
            .await?
            .into_iter()
            .find(|source| source.id == id))
    }

    pub async fn save(&self, id: Option<Uuid>, mut source: Source) -> Result<Uuid, Error> {
        self.ensure_names().await?;
        Config {
            sources: vec![source.clone()],
        }
        .validate()?;
        if matches!(
            &source.transport,
            Transport::McpHttp {
                bearer_env: Some(_),
                ..
            }
        ) || matches!(&source.transport,Transport::McpStdio{env,..} if !env.is_empty())
        {
            return Err(Error::Config("use a linked account for credentials; environment references are only supported by the one-time file import".into()));
        }
        if let Transport::OpenApi { url, server_url } = &mut source.transport {
            *server_url = Some(match server_url {
                Some(value) => super::super::openapi::server_url(value)?.to_string(),
                None => {
                    self.load_openapi(url.clone(), None, stubs::identifier(&source.slug))
                        .await?
                        .server
                }
            });
        }
        let mut tx = self.db.0.begin().await.map_err(DbError::from)?;
        sqlx::query!("LOCK TABLE sources IN SHARE ROW EXCLUSIVE MODE")
            .execute(&mut *tx)
            .await
            .map_err(DbError::from)?;
        let existing=match id {
            Some(id)=>Some(sqlx::query!(r#"SELECT config AS "config: sqlx::types::Json<Source>" FROM sources WHERE id=$1 FOR UPDATE"#,id).fetch_optional(&mut *tx).await.map_err(DbError::from)?.ok_or(Error::NotFound)?.config.0),
            None=>{
                let count=sqlx::query_scalar!("SELECT count(*) AS \"count!\" FROM sources").fetch_one(&mut *tx).await.map_err(DbError::from)?;
                if count>=64 {return Err(Error::Config("maximum 64 sources".into()));}
                None
            }
        };
        let id = id.unwrap_or_else(Uuid::new_v4);
        let existing_name = sqlx::query_scalar!("SELECT python_name FROM sources WHERE id=$1", id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(DbError::from)?
            .flatten();
        let names = sqlx::query_scalar!(
            "SELECT python_name AS \"python_name!\" FROM sources WHERE python_name IS NOT NULL"
        )
        .fetch_all(&mut *tx)
        .await
        .map_err(DbError::from)?;
        let mut used = names.into_iter().collect();
        let python_name = existing_name.unwrap_or_else(|| available_name(&source.slug, &mut used));
        if existing
            .as_ref()
            .is_some_and(|old| old.transport != source.transport)
        {
            sqlx::query!("DELETE FROM tool_policies WHERE source_id=$1", id)
                .execute(&mut *tx)
                .await
                .map_err(DbError::from)?;
            sqlx::query!("DELETE FROM endpoint_tools WHERE source_id=$1", id)
                .execute(&mut *tx)
                .await
                .map_err(DbError::from)?;
            // Credentials belong to the original destination, never to a replacement URL or command.
            sqlx::query!("DELETE FROM accounts WHERE source_id=$1", id)
                .execute(&mut *tx)
                .await
                .map_err(DbError::from)?;
        }
        let config =
            serde_json::to_value(&source).map_err(|_| Error::Config("invalid source".into()))?;
        let kind = match source.transport {
            Transport::McpHttp { .. } => "mcp_http",
            Transport::McpStdio { .. } => "mcp_stdio",
            Transport::OpenApi { .. } => "openapi",
            Transport::Graphql { .. } => "graphql",
        };
        sqlx::query!("INSERT INTO sources(id,slug,kind,config,python_name) VALUES($1,$2,$3,$4,$5) ON CONFLICT(id) DO UPDATE SET slug=EXCLUDED.slug,kind=EXCLUDED.kind,config=EXCLUDED.config,revision=sources.revision+1,status='pending',last_error=NULL",id,source.slug,kind,config,python_name).execute(&mut *tx).await.map_err(|error|match error {sqlx::Error::Database(ref e) if e.is_unique_violation()=>Error::Conflict,other=>Error::Database(DbError::from(other))})?;
        tx.commit().await.map_err(DbError::from)?;
        self.wake();
        Ok(id)
    }

    pub async fn remove(&self, id: Uuid) -> Result<bool, Error> {
        let removed = sqlx::query!("DELETE FROM sources WHERE id=$1", id)
            .execute(&self.db.0)
            .await
            .map_err(DbError::from)?
            .rows_affected()
            > 0;
        self.tools
            .write()
            .await
            .retain(|_, tool| tool.source_id != id);
        self.wake();
        Ok(removed)
    }

    pub async fn resync(&self, id: Uuid) -> Result<(), Error> {
        if sqlx::query!(
            "UPDATE sources SET revision=revision+1,status='pending',last_error=NULL WHERE id=$1",
            id
        )
        .execute(&self.db.0)
        .await
        .map_err(DbError::from)?
        .rows_affected()
            == 0
        {
            return Err(Error::NotFound);
        }
        self.wake();
        Ok(())
    }

    pub async fn import_file(&self, path: &Path) -> Result<(), Error> {
        if self.imported().await? {
            return Ok(());
        }
        self.import(Config::read(path).await?).await
    }

    async fn ensure_names(&self) -> Result<(), Error> {
        let mut tx = self.db.0.begin().await.map_err(DbError::from)?;
        sqlx::query!("LOCK TABLE sources IN SHARE ROW EXCLUSIVE MODE")
            .execute(&mut *tx)
            .await
            .map_err(DbError::from)?;
        let rows = sqlx::query!("SELECT id,slug,python_name FROM sources ORDER BY slug")
            .fetch_all(&mut *tx)
            .await
            .map_err(DbError::from)?;
        let mut used = rows
            .iter()
            .filter_map(|row| row.python_name.clone())
            .collect();
        for row in rows.into_iter().filter(|row| row.python_name.is_none()) {
            let name = available_name(&row.slug, &mut used);
            sqlx::query!(
                "UPDATE sources SET python_name=$2 WHERE id=$1",
                row.id,
                name
            )
            .execute(&mut *tx)
            .await
            .map_err(DbError::from)?;
        }
        tx.commit().await.map_err(DbError::from)?;
        Ok(())
    }

    async fn imported(&self) -> Result<bool, Error> {
        Ok(
            sqlx::query_scalar!("SELECT EXISTS(SELECT 1 FROM source_import) AS \"imported!\"")
                .fetch_one(&self.db.0)
                .await
                .map_err(DbError::from)?,
        )
    }

    pub async fn import(&self, config: Config) -> Result<(), Error> {
        if self.imported().await? {
            return Ok(());
        }
        config.validate()?;
        for mut source in config.sources {
            let mut bearer = None;
            let mut environment = BTreeMap::new();
            match &mut source.transport {
                Transport::OpenApi { .. } | Transport::Graphql { .. } => {}
                Transport::McpHttp { bearer_env, .. } => {
                    if let Some(reference) = bearer_env.take() {
                        bearer = Some(SecretString::from(std::env::var(reference).map_err(
                            |_| {
                                Error::Config(
                                    "source credential environment reference is missing".into(),
                                )
                            },
                        )?));
                        source.requires_account = true;
                    }
                }
                Transport::McpStdio { env, .. } => {
                    for (name, reference) in std::mem::take(env) {
                        environment.insert(
                            name,
                            SecretString::from(std::env::var(reference).map_err(|_| {
                                Error::Config(
                                    "source credential environment reference is missing".into(),
                                )
                            })?),
                        );
                    }
                    source.requires_account |= !environment.is_empty();
                }
            }
            let id = sqlx::query_scalar!("SELECT id FROM sources WHERE slug=$1", source.slug)
                .fetch_optional(&self.db.0)
                .await
                .map_err(DbError::from)?;
            let id = self.save(id, source).await?;
            if let Some(bearer) = bearer {
                self.accounts.link_bearer(id, bearer).await?;
            }
            if !environment.is_empty() {
                self.accounts.link_environment(id, environment).await?;
            }
        }
        sqlx::query!("INSERT INTO source_import(singleton) VALUES(true) ON CONFLICT DO NOTHING")
            .execute(&self.db.0)
            .await
            .map_err(DbError::from)?;
        self.wake();
        Ok(())
    }
}

fn available_name(slug: &str, used: &mut std::collections::BTreeSet<String>) -> String {
    let base = stubs::identifier(slug);
    let mut name = base.clone();
    let mut suffix = 2;
    while !used.insert(name.clone()) {
        name = format!("{base}_{suffix}");
        suffix += 1;
    }
    name
}
