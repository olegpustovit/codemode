use crate::infrastructure::postgres::{Database, Error as DbError};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum Action {
    Allow,
    Block,
}
impl Action {
    fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Block => "block",
        }
    }
}
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Choice {
    Default,
    Allow,
    Block,
}
impl Choice {
    fn action(self) -> Option<Action> {
        match self {
            Self::Default => None,
            Self::Allow => Some(Action::Allow),
            Self::Block => Some(Action::Block),
        }
    }
}
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("endpoint or tool not found")]
    NotFound,
    #[error("invalid policy query")]
    Invalid,
    #[error(transparent)]
    Database(#[from] DbError),
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolPolicy {
    pub source_id: Uuid,
    pub source_revision: i64,
    pub upstream_name: String,
    pub path: String,
    pub description: String,
    pub signature: String,
    pub read_only: bool,
    pub destructive: bool,
    pub in_scope: bool,
    pub override_action: Option<Action>,
    pub effective_action: Action,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub tools: Vec<ToolPolicy>,
    pub next_cursor: Option<String>,
}
#[derive(Clone)]
pub struct Policies {
    db: Database,
}
impl Policies {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
    pub async fn effective(
        &self,
        workspace: Uuid,
        source: Uuid,
        name: &str,
    ) -> Result<Action, Error> {
        let default = Action::Allow;
        sqlx::query_scalar!(r#"SELECT CASE WHEN NOT e.all_tools AND NOT EXISTS(SELECT 1 FROM endpoint_tools scope WHERE scope.endpoint_id=e.id AND scope.source_id=$2 AND scope.upstream_name=$3) THEN 'block' ELSE COALESCE(p.action,$4) END AS "action!: Action" FROM endpoints e LEFT JOIN tool_policies p ON p.endpoint_id=e.id AND p.source_id=$2 AND p.upstream_name=$3 WHERE e.id=$1"#,workspace,source,name,default.as_str())
            .fetch_optional(&self.db.0).await.map_err(DbError::from)?.ok_or(Error::NotFound)
    }
    pub async fn blocked(&self, workspace: Uuid) -> Result<Vec<String>, Error> {
        Ok(sqlx::query_scalar!("SELECT t.path FROM tools t JOIN endpoints e ON e.id=$1 LEFT JOIN tool_policies p ON p.source_id=t.source_id AND p.upstream_name=t.upstream_name AND p.endpoint_id=e.id WHERE p.action='block' OR (NOT e.all_tools AND NOT EXISTS(SELECT 1 FROM endpoint_tools scope WHERE scope.endpoint_id=e.id AND scope.source_id=t.source_id AND scope.upstream_name=t.upstream_name))",workspace)
            .fetch_all(&self.db.0).await.map_err(DbError::from)?)
    }
    pub async fn list(
        &self,
        endpoint: Uuid,
        query: &str,
        after: Option<&str>,
    ) -> Result<Page, Error> {
        if query.len() > 1024 || after.is_some_and(|v| v.len() > 1024) {
            return Err(Error::Invalid);
        }
        let mut tools=sqlx::query_as!(ToolPolicy,r#"SELECT t.source_id,s.revision AS source_revision,t.upstream_name,t.path,t.description,t.stub_full AS signature,t.read_only,t.destructive,
            (e.all_tools OR EXISTS(SELECT 1 FROM endpoint_tools scope WHERE scope.endpoint_id=e.id AND scope.source_id=t.source_id AND scope.upstream_name=t.upstream_name)) AS "in_scope!",p.action AS "override_action?: Action",
            CASE WHEN NOT e.all_tools AND NOT EXISTS(SELECT 1 FROM endpoint_tools scope WHERE scope.endpoint_id=e.id AND scope.source_id=t.source_id AND scope.upstream_name=t.upstream_name) THEN 'block' ELSE COALESCE(p.action,'allow') END AS "effective_action!: Action"
            FROM tools t JOIN sources s ON s.id=t.source_id JOIN endpoints e ON e.id=$1 LEFT JOIN tool_policies p ON p.endpoint_id=e.id AND p.source_id=t.source_id AND p.upstream_name=t.upstream_name
            WHERE (e.all_sources OR EXISTS(SELECT 1 FROM endpoint_sources es WHERE es.endpoint_id=e.id AND es.source_id=t.source_id))
            AND ($2='' OR t.search @@ websearch_to_tsquery('simple',$2)) AND ($3::text IS NULL OR t.path>$3) ORDER BY t.path LIMIT 51"#,endpoint,query,after)
            .fetch_all(&self.db.0).await.map_err(DbError::from)?;
        let next_cursor = if tools.len() > 50 {
            tools.truncate(50);
            tools.last().map(|t| t.path.clone())
        } else {
            None
        };
        Ok(Page { tools, next_cursor })
    }
    pub async fn set_in_scope(
        &self,
        endpoint: Uuid,
        source: Uuid,
        revision: i64,
        name: &str,
        included: bool,
    ) -> Result<(), Error> {
        if name.is_empty() || name.len() > 1024 {
            return Err(Error::Invalid);
        }
        let mut tx = self.db.0.begin().await.map_err(DbError::from)?;
        sqlx::query!("SELECT id FROM endpoints WHERE id=$1 FOR SHARE", endpoint)
            .fetch_optional(&mut *tx)
            .await
            .map_err(DbError::from)?
            .ok_or(Error::NotFound)?;
        sqlx::query!("SELECT t.source_id FROM tools t JOIN sources s ON s.id=t.source_id JOIN endpoints e ON e.id=$1 WHERE s.id=$2 AND s.revision=$3 AND t.upstream_name=$4 AND (e.all_sources OR EXISTS(SELECT 1 FROM endpoint_sources es WHERE es.endpoint_id=e.id AND es.source_id=t.source_id)) FOR SHARE OF s",endpoint,source,revision,name).fetch_optional(&mut *tx).await.map_err(DbError::from)?.ok_or(Error::NotFound)?;
        if included {
            sqlx::query!("INSERT INTO endpoint_tools(endpoint_id,source_id,upstream_name) VALUES($1,$2,$3) ON CONFLICT DO NOTHING",endpoint,source,name).execute(&mut *tx).await.map_err(DbError::from)?;
        } else {
            sqlx::query!("DELETE FROM endpoint_tools WHERE endpoint_id=$1 AND source_id=$2 AND upstream_name=$3",endpoint,source,name).execute(&mut *tx).await.map_err(DbError::from)?;
        }
        tx.commit().await.map_err(DbError::from)?;
        Ok(())
    }
    pub async fn set_source(
        &self,
        endpoint: Uuid,
        source: Uuid,
        revision: i64,
        action: Action,
    ) -> Result<(), Error> {
        let mut tx = self.db.0.begin().await.map_err(DbError::from)?;
        sqlx::query!("SELECT id FROM endpoints WHERE id=$1 FOR SHARE", endpoint)
            .fetch_optional(&mut *tx)
            .await
            .map_err(DbError::from)?
            .ok_or(Error::NotFound)?;
        sqlx::query!("SELECT s.id FROM sources s JOIN endpoints e ON e.id=$1 WHERE s.id=$2 AND s.revision=$3 AND (e.all_sources OR EXISTS(SELECT 1 FROM endpoint_sources es WHERE es.endpoint_id=e.id AND es.source_id=s.id)) FOR SHARE OF s",endpoint,source,revision)
            .fetch_optional(&mut *tx).await.map_err(DbError::from)?.ok_or(Error::NotFound)?;
        sqlx::query!("INSERT INTO tool_policies(endpoint_id,source_id,upstream_name,action) SELECT $1,source_id,upstream_name,$3 FROM tools WHERE source_id=$2 ON CONFLICT(endpoint_id,source_id,upstream_name) DO UPDATE SET action=EXCLUDED.action",endpoint,source,action.as_str())
            .execute(&mut *tx).await.map_err(DbError::from)?;
        tx.commit().await.map_err(DbError::from)?;
        Ok(())
    }
    pub async fn set(
        &self,
        endpoint: Uuid,
        source: Uuid,
        revision: i64,
        name: &str,
        choice: Choice,
    ) -> Result<(), Error> {
        if name.len() > 1024 {
            return Err(Error::Invalid);
        }
        let mut tx = self.db.0.begin().await.map_err(DbError::from)?;
        sqlx::query!("SELECT id FROM endpoints WHERE id=$1 FOR SHARE", endpoint)
            .fetch_optional(&mut *tx)
            .await
            .map_err(DbError::from)?
            .ok_or(Error::NotFound)?;
        let tool=sqlx::query!("SELECT t.source_id,t.upstream_name FROM tools t JOIN sources s ON s.id=t.source_id JOIN endpoints e ON e.id=$1 WHERE s.id=$2 AND s.revision=$3 AND t.upstream_name=$4 AND (e.all_sources OR EXISTS(SELECT 1 FROM endpoint_sources es WHERE es.endpoint_id=e.id AND es.source_id=t.source_id)) FOR SHARE OF s",endpoint,source,revision,name)
            .fetch_optional(&mut *tx).await.map_err(DbError::from)?.ok_or(Error::NotFound)?;
        match choice.action() {
            Some(action) => {
                sqlx::query!("INSERT INTO tool_policies(endpoint_id,source_id,upstream_name,action) VALUES($1,$2,$3,$4) ON CONFLICT(endpoint_id,source_id,upstream_name) DO UPDATE SET action=EXCLUDED.action",endpoint,tool.source_id,tool.upstream_name,action.as_str()).execute(&mut *tx).await.map_err(DbError::from)?;
            }
            None => {
                sqlx::query!("DELETE FROM tool_policies WHERE endpoint_id=$1 AND source_id=$2 AND upstream_name=$3",endpoint,tool.source_id,tool.upstream_name).execute(&mut *tx).await.map_err(DbError::from)?;
            }
        }
        tx.commit().await.map_err(DbError::from)?;
        Ok(())
    }
}
