use super::*;
use secrecy::SecretString;

#[derive(Deserialize, rmcp::schemars::JsonSchema)]
pub struct SourceInput {
    #[serde(flatten)]
    pub source: Source,
    pub name: Option<String>,
    pub icon: Option<String>,
}

#[derive(Deserialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Display {
    pub name: String,
    pub icon: String,
}
impl Display {
    fn validate(&self) -> Result<(), Error> {
        if self.name.trim().is_empty()
            || self.name.chars().count() > 120
            || self.name.chars().any(char::is_control)
        {
            return Err(Error::Display(
                "name must contain 1..120 visible characters",
            ));
        }
        if ![
            "plug", "database", "cloud", "bolt", "chart", "mail", "calendar", "card", "box", "bug",
            "book", "globe", "terminal", "key", "users", "file", "mono",
        ]
        .contains(&self.icon.as_str())
        {
            return Err(Error::Display("unknown source icon"));
        }
        Ok(())
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    pub id: Uuid,
    pub name: String,
    pub icon: String,
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
        let presets = super::super::providers::registry()
            .map_err(|_| Error::Config("invalid provider registry".into()))?;
        let rows = sqlx::query!(
            r#"SELECT s.id,s.name,s.icon,s.config AS "config: sqlx::types::Json<Source>",s.status,s.last_error,
            to_char(s.synced_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS synced_at,
            (SELECT count(*) FROM tools t WHERE t.source_id=s.id) AS "tool_count!",s.revision,s.python_name AS "python_name!"
            FROM sources s ORDER BY s.slug LIMIT 64"#
        )
        .fetch_all(&self.db.0)
        .await
        .map_err(DbError::from)?;
        let linked: std::collections::BTreeSet<_> = self
            .accounts
            .list()
            .await?
            .into_iter()
            .map(|account| account.source_id)
            .collect();
        Ok(rows
            .into_iter()
            .map(|row| {
                let needs_link = row.config.0.requires_account && !linked.contains(&row.id);
                let preset = presets.iter().find(|entry| matches!(&row.config.0.transport,Transport::McpHttp{url,..} if entry.url.trim_end_matches('/') == url.trim_end_matches('/') && !row.config.0.slug.starts_with(&format!("{}_", entry.slug))));
                Record {
                    id: row.id,
                    name: preset.map_or(row.name, |entry| entry.name.clone()),
                    icon: preset.map_or(row.icon, |entry| entry.icon.clone()),
                    source: row.config.0,
                    status: if needs_link {
                        "needs_link".into()
                    } else {
                        row.status
                    },
                    last_error: if needs_link { None } else { row.last_error },
                    synced_at: row.synced_at,
                    tool_count: row.tool_count,
                    revision: row.revision,
                    python_name: row.python_name,
                }
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

    pub async fn save(&self, id: Option<Uuid>, source: Source) -> Result<Uuid, Error> {
        self.save_input(
            id,
            SourceInput {
                source,
                name: None,
                icon: None,
            },
        )
        .await
    }

    pub async fn save_input(&self, id: Option<Uuid>, input: SourceInput) -> Result<Uuid, Error> {
        let SourceInput {
            mut source,
            name,
            icon,
        } = input;
        if (name.is_some() || icon.is_some())
            && matches!(&source.transport,Transport::McpHttp{url,..} if super::super::providers::registry().map_err(|_| Error::Config("invalid provider registry".into()))?.iter().any(|entry| entry.url.trim_end_matches('/') == url.trim_end_matches('/') && !source.slug.starts_with(&format!("{}_", entry.slug))))
        {
            return Err(Error::Display(
                "preset names and icons are fixed; add this provider from Integrations",
            ));
        }
        let display = Display {
            name: name.clone().unwrap_or_else(|| source.slug.clone()),
            icon: icon.clone().unwrap_or_else(|| "plug".into()),
        };
        display.validate()?;
        if id.is_none() {
            source.slug.make_ascii_lowercase();
        }
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
        if let Some(old) = &existing {
            // Display edits cannot rename a connector's stable Python namespace or slug.
            source.slug.clone_from(&old.slug);
        }
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
        sqlx::query!("INSERT INTO sources(id,slug,kind,config,python_name,name,icon) VALUES($1,$2,$3,$4,$5,$6,$7) ON CONFLICT(id) DO UPDATE SET kind=EXCLUDED.kind,config=EXCLUDED.config,name=COALESCE($8,sources.name),icon=COALESCE($9,sources.icon),revision=sources.revision+1,status='pending',last_error=NULL",id,source.slug,kind,config,python_name,display.name,display.icon,name,icon).execute(&mut *tx).await.map_err(|error|match error {sqlx::Error::Database(ref e) if e.is_unique_violation()=>Error::Conflict,other=>Error::Database(DbError::from(other))})?;
        tx.commit().await.map_err(DbError::from)?;
        self.wake();
        Ok(id)
    }

    pub async fn set_display(&self, id: Uuid, display: Display) -> Result<(), Error> {
        display.validate()?;
        let record = self.get(id).await?.ok_or(Error::NotFound)?;
        if let Transport::McpHttp { url, .. } = &record.source.transport {
            if super::super::providers::registry()
                .map_err(|_| Error::Config("invalid provider registry".into()))?
                .iter()
                .any(|entry| {
                    entry.url.trim_end_matches('/') == url.trim_end_matches('/')
                        && !record.source.slug.starts_with(&format!("{}_", entry.slug))
                })
            {
                return Err(Error::Display("preset names and icons are fixed"));
            }
        }
        if sqlx::query!(
            "UPDATE sources SET name=$2,icon=$3 WHERE id=$1",
            id,
            display.name,
            display.icon
        )
        .execute(&self.db.0)
        .await
        .map_err(DbError::from)?
        .rows_affected()
            == 0
        {
            return Err(Error::NotFound);
        }
        Ok(())
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

impl Sources {
    pub async fn sync_many(
        &self,
        selection: super::super::bulk::Selection<Uuid>,
        expected: i64,
    ) -> Result<u64, super::super::bulk::Error> {
        use super::super::bulk::Error;
        selection.validate(expected)?;
        let patterns = selection.patterns();
        let vendors: Vec<_> = super::super::providers::registry()
            .map_err(|_| Error::Invalid)?
            .into_iter()
            .map(|entry| entry.url.trim_end_matches('/').to_owned())
            .collect();
        let mut tx = self.db.0.begin().await.map_err(DbError::from)?;
        let targets=sqlx::query_scalar!("SELECT id FROM sources WHERE ($1::uuid[] IS NULL OR id=ANY($1)) AND NOT(kind='mcp_http' AND rtrim(config->>'url','/')=ANY($2)) AND NOT EXISTS(SELECT 1 FROM unnest($3::text[]) terms(pattern) WHERE NOT(name ILIKE pattern)) ORDER BY id FOR UPDATE",selection.ids.as_deref(),&vendors,&patterns).fetch_all(&mut *tx).await.map_err(DbError::from)?;
        if targets.len() as i64 != expected {
            return Err(Error::Changed);
        }
        let request = Uuid::new_v4();
        let changed=sqlx::query!("WITH changed AS (UPDATE sources SET revision=revision+1,status='pending',last_error=NULL WHERE id=ANY($1) RETURNING id) INSERT INTO control_changes(request_id,kind,item,action) SELECT $2,'source',id::text,'sync' FROM changed",&targets,request).execute(&mut *tx).await.map_err(DbError::from)?.rows_affected();
        tx.commit().await.map_err(DbError::from)?;
        self.wake();
        Ok(changed)
    }
}
