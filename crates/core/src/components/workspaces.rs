use crate::infrastructure::postgres::{Database, Error as DbError};
use std::collections::BTreeSet;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("workspace name must contain 1..80 characters")]
    Invalid,
    #[error("the default workspace cannot be removed or have its source membership changed")]
    Reserved,
    #[error("workspace name already exists")]
    Conflict,
    #[error("workspace does not exist")]
    NotFound,
    #[error("workspace contains unknown or duplicate sources")]
    Sources,
    #[error("maximum 64 custom workspaces")]
    Limit,
    #[error(transparent)]
    Database(#[from] DbError),
}

#[derive(Clone)]
pub struct Workspaces {
    db: Database,
    main: String,
}
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Workspace {
    pub id: Uuid,
    pub name: String,
    pub all_sources: bool,
    pub all_tools: bool,
    pub source_ids: Vec<Uuid>,
}
impl Workspaces {
    pub fn with_database(&self, db: Database) -> Self {
        Self { db, ..self.clone() }
    }
    pub fn new(db: Database, main: String) -> Result<Self, Error> {
        if authority(&main).as_deref() != Some(&main) {
            return Err(Error::Invalid);
        }
        Ok(Self { db, main })
    }
    pub fn main_hostname(&self) -> &str {
        &self.main
    }
    pub async fn initialize(&self) -> Result<(), Error> {
        sqlx::query!("INSERT INTO endpoints(slug,hostname,all_sources) VALUES('All sources',$1,true) ON CONFLICT(tenant_id) WHERE all_sources DO NOTHING",self.main)
            .execute(&self.db.0).await.map_err(DbError::from)?;
        Ok(())
    }
    pub async fn list(&self) -> Result<Vec<Workspace>, Error> {
        Ok(sqlx::query_as!(Workspace,r#"SELECT e.id,e.slug AS name,e.all_sources,e.all_tools,
            ARRAY(SELECT source_id FROM endpoint_sources WHERE endpoint_id=e.id ORDER BY source_id) AS "source_ids!"
            FROM endpoints e ORDER BY e.all_sources DESC,e.slug LIMIT 65"#).fetch_all(&self.db.0).await.map_err(DbError::from)?)
    }
    pub async fn get(&self, id: Uuid) -> Result<Option<Workspace>, Error> {
        Ok(sqlx::query_as!(Workspace,r#"SELECT e.id,e.slug AS name,e.all_sources,e.all_tools,
            ARRAY(SELECT source_id FROM endpoint_sources WHERE endpoint_id=e.id ORDER BY source_id) AS "source_ids!"
            FROM endpoints e WHERE e.id=$1"#,id).fetch_optional(&self.db.0).await.map_err(DbError::from)?)
    }
    pub async fn default_id(&self) -> Result<Uuid, Error> {
        sqlx::query_scalar!("SELECT id FROM endpoints WHERE all_sources")
            .fetch_optional(&self.db.0)
            .await
            .map_err(DbError::from)?
            .ok_or(Error::NotFound)
    }
    pub async fn create(&self, name: &str, source_ids: &[Uuid]) -> Result<Uuid, Error> {
        validate_name(name)?;
        validate_sources(source_ids)?;
        let mut tx = self.db.0.begin().await.map_err(DbError::from)?;
        sqlx::query!("LOCK TABLE endpoints IN SHARE ROW EXCLUSIVE MODE")
            .execute(&mut *tx)
            .await
            .map_err(DbError::from)?;
        let count = sqlx::query_scalar!(
            "SELECT count(*) AS \"count!\" FROM endpoints WHERE NOT all_sources"
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(DbError::from)?;
        if count >= 64 {
            return Err(Error::Limit);
        }
        let id = sqlx::query_scalar!(
            "INSERT INTO endpoints(slug,hostname) VALUES($1,$2) RETURNING id",
            name.trim(),
            self.main
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(map_db)?;
        let inserted=sqlx::query!("INSERT INTO endpoint_sources(endpoint_id,source_id) SELECT $1,id FROM sources WHERE id=ANY($2)",id,source_ids).execute(&mut *tx).await.map_err(DbError::from)?.rows_affected();
        if inserted != source_ids.len() as u64 {
            return Err(Error::Sources);
        }
        tx.commit().await.map_err(DbError::from)?;
        Ok(id)
    }
    pub async fn update(&self, id: Uuid, name: &str, source_ids: &[Uuid]) -> Result<(), Error> {
        validate_name(name)?;
        validate_sources(source_ids)?;
        let mut tx = self.db.0.begin().await.map_err(DbError::from)?;
        let main = sqlx::query_scalar!(
            "SELECT all_sources FROM endpoints WHERE id=$1 FOR UPDATE",
            id
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(DbError::from)?
        .ok_or(Error::NotFound)?;
        if main {
            return Err(Error::Reserved);
        }
        sqlx::query!("UPDATE endpoints SET slug=$2 WHERE id=$1", id, name.trim())
            .execute(&mut *tx)
            .await
            .map_err(map_db)?;
        sqlx::query!("DELETE FROM endpoint_sources WHERE endpoint_id=$1", id)
            .execute(&mut *tx)
            .await
            .map_err(DbError::from)?;
        let inserted=sqlx::query!("INSERT INTO endpoint_sources(endpoint_id,source_id) SELECT $1,id FROM sources WHERE id=ANY($2)",id,source_ids).execute(&mut *tx).await.map_err(DbError::from)?.rows_affected();
        if inserted != source_ids.len() as u64 {
            return Err(Error::Sources);
        }
        tx.commit().await.map_err(DbError::from)?;
        Ok(())
    }
    pub async fn set_all_tools(&self, id: Uuid, all: bool) -> Result<(), Error> {
        if sqlx::query!("UPDATE endpoints SET all_tools=$2 WHERE id=$1", id, all)
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
        let mut tx = self.db.0.begin().await.map_err(DbError::from)?;
        let main = sqlx::query_scalar!(
            "SELECT all_sources FROM endpoints WHERE id=$1 FOR UPDATE",
            id
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(DbError::from)?;
        let Some(main) = main else { return Ok(false) };
        if main {
            return Err(Error::Reserved);
        }
        sqlx::query!(
            "UPDATE access_tokens SET revoked_at=COALESCE(revoked_at,now()) WHERE workspace_id=$1",
            id
        )
        .execute(&mut *tx)
        .await
        .map_err(DbError::from)?;
        sqlx::query!("DELETE FROM endpoints WHERE id=$1", id)
            .execute(&mut *tx)
            .await
            .map_err(DbError::from)?;
        tx.commit().await.map_err(DbError::from)?;
        Ok(true)
    }
    pub async fn sources(&self, workspace: Uuid) -> Result<Option<Vec<Uuid>>, Error> {
        let row = self.get(workspace).await?.ok_or(Error::NotFound)?;
        Ok(if row.all_sources {
            None
        } else {
            Some(row.source_ids)
        })
    }
}
fn validate_name(name: &str) -> Result<(), Error> {
    if name.trim().is_empty()
        || name.trim().chars().count() > 80
        || name.chars().any(char::is_control)
    {
        return Err(Error::Invalid);
    }
    Ok(())
}
fn validate_sources(ids: &[Uuid]) -> Result<(), Error> {
    if ids.len() > 64 || ids.iter().collect::<BTreeSet<_>>().len() != ids.len() {
        return Err(Error::Sources);
    }
    Ok(())
}
fn map_db(error: sqlx::Error) -> Error {
    match error {
        sqlx::Error::Database(ref e) if e.is_unique_violation() => Error::Conflict,
        other => Error::Database(DbError::from(other)),
    }
}
pub fn authority(value: &str) -> Option<String> {
    let host = value.strip_suffix(":443").unwrap_or(value);
    let host = host.strip_suffix('.').unwrap_or(host).to_ascii_lowercase();
    (!host.is_empty()
        && host.len() <= 253
        && host.split('.').all(|part| {
            !part.is_empty()
                && part.len() <= 63
                && !part.starts_with('-')
                && !part.ends_with('-')
                && part.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
        }))
    .then_some(host)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn workspace_names_are_human_names_and_authorities_cannot_smuggle_ports() {
        assert!(validate_name("My research workspace").is_ok());
        assert!(validate_name("\nsecret").is_err());
        assert!(validate_name(" ").is_err());
        assert_eq!(
            authority("CODEMODE.example:443"),
            Some("codemode.example".into())
        );
        for invalid in [
            "",
            "work.example:444",
            "work.example:443:443",
            "work.example/path",
            "user@work.example",
            "work..example",
            "-work.example",
        ] {
            assert!(authority(invalid).is_none(), "{invalid}");
        }
    }
}
