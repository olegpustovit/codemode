mod recipes;
use crate::infrastructure::postgres::{Database, Error as DbError};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("saved script or endpoint not found")]
    NotFound,
    #[error(
        "script name must contain 1..120 characters, description at most 2000, and Python source at most 64 KiB"
    )]
    Invalid,
    #[error("a script with this name already exists in the endpoint")]
    Conflict,
    #[error("script changed; copy your edits before reloading the latest version")]
    Changed,
    #[error("maximum 128 saved scripts per endpoint")]
    Limit,
    #[error("saving a global script needs administration access")]
    Permission,
    #[error(transparent)]
    Database(#[from] DbError),
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Content {
    pub name: String,
    pub description: String,
    pub code: String,
}
impl Content {
    fn validate(&mut self) -> Result<(), Error> {
        self.name = self.name.trim().to_owned();
        if self.name.is_empty()
            || self.name.chars().count() > 120
            || self.name.chars().any(char::is_control)
            || self.description.chars().count() > 2000
            || self.code.len() > 65536
            || self.code.contains('\0')
            || self.description.contains('\0')
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Scope {
    Workspace,
    Global,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub revision: i64,
    pub updated_at: String,
    pub workspace_id: Option<Uuid>,
    pub saved_by: Option<String>,
    pub from_execution: Option<Uuid>,
}
#[derive(Serialize)]
pub struct Script {
    #[serde(flatten)]
    pub summary: Summary,
    pub code: String,
}
#[derive(Serialize)]
pub struct Match {
    pub id: Uuid,
    pub name: String,
    pub description: String,
}
#[derive(Clone)]
pub struct Scripts {
    db: Database,
}
impl Scripts {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
    pub async fn list(&self, endpoint: Uuid) -> Result<Vec<Summary>, Error> {
        self.install_recipes(endpoint).await?;
        Ok(sqlx::query_as!(Summary,r#"SELECT id,name,description,revision,endpoint_id AS workspace_id,saved_by,from_execution,to_char(updated_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS "updated_at!" FROM saved_scripts WHERE tenant_id=(SELECT tenant_id FROM endpoints WHERE id=$1) AND (endpoint_id=$1 OR endpoint_id IS NULL) ORDER BY name,id"#,endpoint).fetch_all(&self.db.0).await.map_err(DbError::from)?)
    }
    pub async fn get(&self, endpoint: Uuid, id: Uuid) -> Result<Script, Error> {
        let row=sqlx::query!(r#"SELECT id,name,description,code,revision,endpoint_id AS workspace_id,saved_by,from_execution,to_char(updated_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS "updated_at!" FROM saved_scripts WHERE tenant_id=(SELECT tenant_id FROM endpoints WHERE id=$1) AND (endpoint_id=$1 OR endpoint_id IS NULL) AND id=$2"#,endpoint,id).fetch_optional(&self.db.0).await.map_err(DbError::from)?.ok_or(Error::NotFound)?;
        Ok(Script {
            summary: Summary {
                id: row.id,
                name: row.name,
                description: row.description,
                revision: row.revision,
                updated_at: row.updated_at,
                workspace_id: row.workspace_id,
                saved_by: row.saved_by,
                from_execution: row.from_execution,
            },
            code: row.code,
        })
    }
    pub async fn search(&self, workspace: Uuid, query: &str) -> Result<Vec<Match>, Error> {
        if query.len() > 1024 {
            return Err(Error::Invalid);
        }
        self.install_recipes(workspace).await?;
        Ok(sqlx::query_as!(Match,"SELECT s.id,s.name,s.description FROM saved_scripts s JOIN endpoints e ON e.tenant_id=s.tenant_id WHERE e.id=$1 AND (s.endpoint_id=e.id OR s.endpoint_id IS NULL) AND ($2='' OR s.search @@ websearch_to_tsquery('simple',$2)) ORDER BY ts_rank(s.search,websearch_to_tsquery('simple',$2)) DESC,s.name,s.id LIMIT 20",workspace,query).fetch_all(&self.db.0).await.map_err(DbError::from)?)
    }
    pub async fn code_for(&self, workspace: Uuid, id: Uuid) -> Result<String, Error> {
        sqlx::query_scalar!("SELECT s.code FROM saved_scripts s JOIN endpoints e ON e.tenant_id=s.tenant_id WHERE e.id=$1 AND (s.endpoint_id=e.id OR s.endpoint_id IS NULL) AND s.id=$2",workspace,id).fetch_optional(&self.db.0).await.map_err(DbError::from)?.ok_or(Error::NotFound)
    }
    pub async fn save(
        &self,
        endpoint: Uuid,
        existing: Option<(Uuid, i64)>,
        mut content: Content,
    ) -> Result<Script, Error> {
        content.validate()?;
        let mut tx = self.db.0.begin().await.map_err(DbError::from)?;
        sqlx::query!("SELECT id FROM endpoints WHERE id=$1 FOR UPDATE", endpoint)
            .fetch_optional(&mut *tx)
            .await
            .map_err(DbError::from)?
            .ok_or(Error::NotFound)?;
        let id = match existing {
            Some((id, revision)) => {
                let current = sqlx::query_scalar!(
                    "SELECT revision FROM saved_scripts WHERE id=$1 AND tenant_id=(SELECT tenant_id FROM endpoints WHERE id=$2) AND (endpoint_id=$2 OR endpoint_id IS NULL) FOR UPDATE",
                    id,
                    endpoint
                )
                .fetch_optional(&mut *tx)
                .await
                .map_err(DbError::from)?
                .ok_or(Error::NotFound)?;
                if current != revision {
                    return Err(Error::Changed);
                }
                sqlx::query!("UPDATE saved_scripts SET name=$3,description=$4,code=$5,revision=revision+1,updated_at=now() WHERE id=$1 AND tenant_id=(SELECT tenant_id FROM endpoints WHERE id=$2) AND (endpoint_id=$2 OR endpoint_id IS NULL)",id,endpoint,content.name,content.description,content.code).execute(&mut *tx).await.map_err(database_error)?;
                id
            }
            None => {
                let count = sqlx::query_scalar!(
                    "SELECT count(*) AS \"count!\" FROM saved_scripts WHERE endpoint_id=$1",
                    endpoint
                )
                .fetch_one(&mut *tx)
                .await
                .map_err(DbError::from)?;
                if count >= 128 {
                    return Err(Error::Limit);
                }
                sqlx::query_scalar!("INSERT INTO saved_scripts(endpoint_id,name,description,code) VALUES($1,$2,$3,$4) RETURNING id",endpoint,content.name,content.description,content.code).fetch_one(&mut *tx).await.map_err(database_error)?
            }
        };
        let row=sqlx::query!(r#"SELECT id,name,description,code,revision,endpoint_id AS workspace_id,saved_by,from_execution,to_char(updated_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS "updated_at!" FROM saved_scripts WHERE tenant_id=(SELECT tenant_id FROM endpoints WHERE id=$1) AND (endpoint_id=$1 OR endpoint_id IS NULL) AND id=$2"#,endpoint,id).fetch_one(&mut *tx).await.map_err(DbError::from)?;
        tx.commit().await.map_err(DbError::from)?;
        Ok(Script {
            summary: Summary {
                id: row.id,
                name: row.name,
                description: row.description,
                revision: row.revision,
                updated_at: row.updated_at,
                workspace_id: row.workspace_id,
                saved_by: row.saved_by,
                from_execution: row.from_execution,
            },
            code: row.code,
        })
    }
    pub(crate) async fn save_from_execution(
        &self,
        workspace: Uuid,
        execution: Uuid,
        scope: Scope,
        mut content: Content,
    ) -> Result<Uuid, Error> {
        content.validate()?;
        let mut tx = self.db.0.begin().await.map_err(DbError::from)?;
        let tenant = sqlx::query_scalar!(
            "SELECT tenant_id FROM endpoints WHERE id=$1 FOR UPDATE",
            workspace
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(DbError::from)?
        .ok_or(Error::NotFound)?;
        let actor = sqlx::query!(r#"SELECT left(t.name,120) AS "name!",t.admin FROM executions e JOIN access_tokens t ON t.id=e.token_id WHERE e.id=$1 AND t.workspace_id=$2 AND t.tenant_id=$3 AND t.revoked_at IS NULL"#,execution,workspace,tenant)
            .fetch_optional(&mut *tx).await.map_err(DbError::from)?.ok_or(Error::NotFound)?;
        let target = match scope {
            Scope::Workspace => Some(workspace),
            Scope::Global if actor.admin => None,
            Scope::Global => return Err(Error::Permission),
        };
        sqlx::query!(
            "SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))",
            tenant.to_string()
        )
        .execute(&mut *tx)
        .await
        .map_err(DbError::from)?;
        let existing = sqlx::query_scalar!("SELECT id FROM saved_scripts WHERE tenant_id=$1 AND endpoint_id IS NOT DISTINCT FROM $2 AND name=$3 FOR UPDATE",tenant,target,content.name)
            .fetch_optional(&mut *tx).await.map_err(DbError::from)?;
        let id = if let Some(id) = existing {
            sqlx::query!("UPDATE saved_scripts SET description=$2,code=$3,saved_by=$4,from_execution=$5,revision=revision+1,updated_at=now() WHERE id=$1",id,content.description,content.code,actor.name,execution)
                .execute(&mut *tx).await.map_err(database_error)?;
            id
        } else {
            let count = sqlx::query_scalar!(r#"SELECT count(*) AS "count!" FROM saved_scripts WHERE tenant_id=$1 AND endpoint_id IS NOT DISTINCT FROM $2"#,tenant,target)
                .fetch_one(&mut *tx).await.map_err(DbError::from)?;
            if count >= 128 {
                return Err(Error::Limit);
            }
            sqlx::query_scalar!("INSERT INTO saved_scripts(tenant_id,endpoint_id,name,description,code,saved_by,from_execution) VALUES($1,$2,$3,$4,$5,$6,$7) RETURNING id",tenant,target,content.name,content.description,content.code,actor.name,execution)
                .fetch_one(&mut *tx).await.map_err(database_error)?
        };
        tx.commit().await.map_err(DbError::from)?;
        Ok(id)
    }

    pub async fn set_scope(
        &self,
        workspace: Uuid,
        id: Uuid,
        revision: i64,
        global: bool,
    ) -> Result<(), Error> {
        let mut tx = self.db.0.begin().await.map_err(DbError::from)?;
        let tenant = sqlx::query_scalar!(
            "SELECT tenant_id FROM endpoints WHERE id=$1 FOR UPDATE",
            workspace
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(DbError::from)?
        .ok_or(Error::NotFound)?;
        sqlx::query!(
            "SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))",
            tenant.to_string()
        )
        .execute(&mut *tx)
        .await
        .map_err(DbError::from)?;
        let row = sqlx::query!("SELECT endpoint_id,revision FROM saved_scripts WHERE tenant_id=$1 AND id=$2 AND (endpoint_id=$3 OR endpoint_id IS NULL) FOR UPDATE", tenant,id,workspace)
            .fetch_optional(&mut *tx).await.map_err(DbError::from)?.ok_or(Error::NotFound)?;
        if row.revision != revision {
            return Err(Error::Changed);
        }
        let target = (!global).then_some(workspace);
        if row.endpoint_id == target {
            return Ok(());
        }
        let count = sqlx::query_scalar!(r#"SELECT count(*) AS "count!" FROM saved_scripts WHERE tenant_id=$1 AND endpoint_id IS NOT DISTINCT FROM $2"#,tenant,target)
            .fetch_one(&mut *tx).await.map_err(DbError::from)?;
        if count >= 128 {
            return Err(Error::Limit);
        }
        sqlx::query!("UPDATE saved_scripts SET endpoint_id=$2,revision=revision+1,updated_at=now() WHERE id=$1",id,target)
            .execute(&mut *tx).await.map_err(database_error)?;
        sqlx::query!("INSERT INTO control_changes(tenant_id,request_id,workspace_id,kind,item,action) VALUES($1,$2,$3,'script',$4,$5)",tenant,Uuid::new_v4(),workspace,id.to_string(),if global {"promote"} else {"demote"})
            .execute(&mut *tx).await.map_err(DbError::from)?;
        tx.commit().await.map_err(DbError::from)?;
        Ok(())
    }
    pub async fn remove(&self, endpoint: Uuid, id: Uuid, revision: i64) -> Result<(), Error> {
        let mut tx = self.db.0.begin().await.map_err(DbError::from)?;
        sqlx::query!("SELECT id FROM endpoints WHERE id=$1 FOR UPDATE", endpoint)
            .fetch_optional(&mut *tx)
            .await
            .map_err(DbError::from)?
            .ok_or(Error::NotFound)?;
        let current = sqlx::query_scalar!(
            "SELECT revision FROM saved_scripts WHERE tenant_id=(SELECT tenant_id FROM endpoints WHERE id=$1) AND (endpoint_id=$1 OR endpoint_id IS NULL) AND id=$2 FOR UPDATE",
            endpoint,
            id
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(DbError::from)?
        .ok_or(Error::NotFound)?;
        if current != revision {
            return Err(Error::Changed);
        }
        sqlx::query!(
            "DELETE FROM saved_scripts WHERE tenant_id=(SELECT tenant_id FROM endpoints WHERE id=$1) AND (endpoint_id=$1 OR endpoint_id IS NULL) AND id=$2",
            endpoint,
            id
        )
        .execute(&mut *tx)
        .await
        .map_err(DbError::from)?;
        tx.commit().await.map_err(DbError::from)?;
        Ok(())
    }
}
fn database_error(error: sqlx::Error) -> Error {
    match error {
        sqlx::Error::Database(ref e) if e.is_unique_violation() => Error::Conflict,
        other => Error::Database(DbError::from(other)),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Revision {
    pub id: Uuid,
    pub revision: i64,
}
impl Scripts {
    pub async fn remove_many(
        &self,
        endpoint: Uuid,
        selection: super::bulk::Selection<Uuid>,
        expected: i64,
        revisions: &[Revision],
    ) -> Result<u64, super::bulk::Error> {
        use super::bulk::Error;
        selection.validate(expected)?;
        if revisions.len() as i64 != expected || revisions.len() > 128 {
            return Err(Error::Invalid);
        }
        let revisions: std::collections::BTreeMap<_, _> = revisions
            .iter()
            .map(|revision| (revision.id, revision.revision))
            .collect();
        if revisions.len() as i64 != expected {
            return Err(Error::Invalid);
        }
        let patterns = selection.patterns();
        let mut tx = self.db.0.begin().await.map_err(DbError::from)?;
        sqlx::query!("SELECT id FROM endpoints WHERE id=$1 FOR UPDATE", endpoint)
            .fetch_optional(&mut *tx)
            .await
            .map_err(DbError::from)?
            .ok_or(Error::Changed)?;
        let targets=sqlx::query!("SELECT id,revision FROM saved_scripts WHERE tenant_id=(SELECT tenant_id FROM endpoints WHERE id=$1) AND (endpoint_id=$1 OR endpoint_id IS NULL) AND ($2::uuid[] IS NULL OR id=ANY($2)) AND NOT EXISTS(SELECT 1 FROM unnest($3::text[]) terms(pattern) WHERE NOT(name ILIKE pattern OR description ILIKE pattern)) ORDER BY id FOR UPDATE",endpoint,selection.ids.as_deref(),&patterns).fetch_all(&mut *tx).await.map_err(DbError::from)?;
        if targets.len() as i64 != expected
            || targets
                .iter()
                .any(|target| revisions.get(&target.id) != Some(&target.revision))
        {
            return Err(Error::Changed);
        }
        let ids: Vec<_> = targets.into_iter().map(|target| target.id).collect();
        let request = Uuid::new_v4();
        let changed=sqlx::query!("WITH changed AS (DELETE FROM saved_scripts WHERE tenant_id=(SELECT tenant_id FROM endpoints WHERE id=$1) AND (endpoint_id=$1 OR endpoint_id IS NULL) AND id=ANY($2) RETURNING id) INSERT INTO control_changes(request_id,workspace_id,kind,item,action) SELECT $3,$1,'script',id::text,'delete' FROM changed",endpoint,&ids,request).execute(&mut *tx).await.map_err(DbError::from)?.rows_affected();
        tx.commit().await.map_err(DbError::from)?;
        Ok(changed)
    }
}
