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
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub revision: i64,
    pub updated_at: String,
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
        Ok(sqlx::query_as!(Summary,r#"SELECT id,name,description,revision,to_char(updated_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS "updated_at!" FROM saved_scripts WHERE endpoint_id=$1 ORDER BY name,id"#,endpoint).fetch_all(&self.db.0).await.map_err(DbError::from)?)
    }
    pub async fn get(&self, endpoint: Uuid, id: Uuid) -> Result<Script, Error> {
        let row=sqlx::query!(r#"SELECT id,name,description,code,revision,to_char(updated_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS "updated_at!" FROM saved_scripts WHERE endpoint_id=$1 AND id=$2"#,endpoint,id).fetch_optional(&self.db.0).await.map_err(DbError::from)?.ok_or(Error::NotFound)?;
        Ok(Script {
            summary: Summary {
                id: row.id,
                name: row.name,
                description: row.description,
                revision: row.revision,
                updated_at: row.updated_at,
            },
            code: row.code,
        })
    }
    pub async fn search(&self, workspace: Uuid, query: &str) -> Result<Vec<Match>, Error> {
        if query.len() > 1024 {
            return Err(Error::Invalid);
        }
        Ok(sqlx::query_as!(Match,"SELECT s.id,s.name,s.description FROM saved_scripts s JOIN endpoints e ON e.id=s.endpoint_id WHERE e.id=$1 AND ($2='' OR s.search @@ websearch_to_tsquery('simple',$2)) ORDER BY ts_rank(s.search,websearch_to_tsquery('simple',$2)) DESC,s.name,s.id LIMIT 20",workspace,query).fetch_all(&self.db.0).await.map_err(DbError::from)?)
    }
    pub async fn code_for(&self, workspace: Uuid, id: Uuid) -> Result<String, Error> {
        sqlx::query_scalar!("SELECT s.code FROM saved_scripts s JOIN endpoints e ON e.id=s.endpoint_id WHERE e.id=$1 AND s.id=$2",workspace,id).fetch_optional(&self.db.0).await.map_err(DbError::from)?.ok_or(Error::NotFound)
    }
    pub async fn save(
        &self,
        endpoint: Uuid,
        existing: Option<(Uuid, i64)>,
        mut content: Content,
    ) -> Result<Script, Error> {
        content.name = content.name.trim().to_owned();
        if content.name.is_empty()
            || content.name.chars().count() > 120
            || content.name.chars().any(char::is_control)
            || content.description.chars().count() > 2000
            || content.code.len() > 65536
            || content.code.contains('\0')
            || content.description.contains('\0')
        {
            return Err(Error::Invalid);
        }
        let mut tx = self.db.0.begin().await.map_err(DbError::from)?;
        sqlx::query!("SELECT id FROM endpoints WHERE id=$1 FOR UPDATE", endpoint)
            .fetch_optional(&mut *tx)
            .await
            .map_err(DbError::from)?
            .ok_or(Error::NotFound)?;
        let id = match existing {
            Some((id, revision)) => {
                let current = sqlx::query_scalar!(
                    "SELECT revision FROM saved_scripts WHERE id=$1 AND endpoint_id=$2",
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
                sqlx::query!("UPDATE saved_scripts SET name=$3,description=$4,code=$5,revision=revision+1,updated_at=now() WHERE id=$1 AND endpoint_id=$2",id,endpoint,content.name,content.description,content.code).execute(&mut *tx).await.map_err(database_error)?;
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
        let row=sqlx::query!(r#"SELECT id,name,description,code,revision,to_char(updated_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS "updated_at!" FROM saved_scripts WHERE endpoint_id=$1 AND id=$2"#,endpoint,id).fetch_one(&mut *tx).await.map_err(DbError::from)?;
        tx.commit().await.map_err(DbError::from)?;
        Ok(Script {
            summary: Summary {
                id: row.id,
                name: row.name,
                description: row.description,
                revision: row.revision,
                updated_at: row.updated_at,
            },
            code: row.code,
        })
    }
    pub async fn remove(&self, endpoint: Uuid, id: Uuid, revision: i64) -> Result<(), Error> {
        let mut tx = self.db.0.begin().await.map_err(DbError::from)?;
        sqlx::query!("SELECT id FROM endpoints WHERE id=$1 FOR UPDATE", endpoint)
            .fetch_optional(&mut *tx)
            .await
            .map_err(DbError::from)?
            .ok_or(Error::NotFound)?;
        let current = sqlx::query_scalar!(
            "SELECT revision FROM saved_scripts WHERE endpoint_id=$1 AND id=$2",
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
            "DELETE FROM saved_scripts WHERE endpoint_id=$1 AND id=$2",
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
