pub mod memo;
use super::{
    auth::Principal,
    execution::{log, title::title},
};
use crate::infrastructure::postgres::{Database, Error as DbError};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("session or its agent grant was not found")]
    NotFound,
    #[error(
        "session title must contain 1–160 characters without control characters; search is limited to 1024 bytes"
    )]
    Invalid,
    #[error(
        "memo is limited to 64 KiB of JSON per agent session; store IDs or small summaries instead of full tool results"
    )]
    MemoLimit,
    #[error(
        "memo accepts non-sensitive JSON only; credential keys and token values are not allowed"
    )]
    MemoSensitive,
    #[error(transparent)]
    Database(#[from] DbError),
}
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub id: Uuid,
    pub token_id: Uuid,
    pub workspace_id: Uuid,
    pub workspace_name: String,
    pub title: String,
    pub client: String,
    pub started_at: String,
    pub last_seen_at: String,
    pub expires_at: String,
    pub executions: i64,
    pub tool_calls: i64,
    pub failures: i64,
    pub status: String,
    pub sources: Vec<String>,
    #[serde(skip)]
    total_matches: i64,
    #[serde(skip)]
    error_matches: i64,
}
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub sessions: Vec<Summary>,
    pub next_cursor: Option<Uuid>,
    pub total_matches: i64,
    pub all_matches: i64,
    pub error_matches: i64,
}
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Detail {
    pub session: Summary,
    pub executions: Vec<log::Detail>,
    pub next_cursor: Option<Uuid>,
}
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Activity {
    pub id: Uuid,
    pub session_id: Uuid,
    pub started_at: String,
    pub status: String,
}
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityPage {
    pub executions: Vec<Activity>,
    pub next_cursor: Option<Uuid>,
}
#[derive(Clone)]
pub struct AgentSessions {
    db: Database,
}
impl AgentSessions {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
    pub(crate) async fn start(&self, principal: &Principal, code: &str) -> Result<Uuid, Error> {
        let mut tx = self.db.0.begin().await.map_err(DbError::from)?;
        let session = self.session(principal, &mut tx).await?;
        let execution=sqlx::query_scalar!("INSERT INTO executions(token_id,code,status,session_id,title,started_at) VALUES($1,$2,'running',$3,$4,clock_timestamp()) RETURNING id",principal.token_id,code,session,title(code,&[])).fetch_one(&mut *tx).await.map_err(DbError::from)?;
        tx.commit().await.map_err(DbError::from)?;
        Ok(execution)
    }
    pub async fn touch(&self, principal: &Principal) -> Result<Uuid, Error> {
        let mut tx = self.db.0.begin().await.map_err(DbError::from)?;
        let session = self.session(principal, &mut tx).await?;
        tx.commit().await.map_err(DbError::from)?;
        Ok(session)
    }
    async fn session(
        &self,
        principal: &Principal,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    ) -> Result<Uuid, Error> {
        let agent=sqlx::query!("SELECT name,oauth_grant FROM access_tokens WHERE id=$1 AND workspace_id=$2 AND resource='/mcp' AND revoked_at IS NULL AND (expires_at IS NULL OR expires_at>now())",principal.token_id,principal.workspace_id).fetch_optional(&mut **tx).await.map_err(DbError::from)?.ok_or(Error::NotFound)?;
        let identity = agent.oauth_grant.unwrap_or(principal.token_id);
        // A separate statement after the lock sees a concurrent start's committed session.
        sqlx::query!(
            "SELECT pg_advisory_xact_lock(hashtextextended($1,0))",
            format!("agent-session:{identity}:{}", principal.workspace_id)
        )
        .execute(&mut **tx)
        .await
        .map_err(DbError::from)?;
        let session=sqlx::query_scalar!(r#"WITH reused AS (
            UPDATE agent_sessions SET last_seen_at=GREATEST(last_seen_at,clock_timestamp())
            WHERE id=(SELECT id FROM agent_sessions WHERE workspace_id=$2
             AND (($3::uuid IS NULL AND token_id=$1) OR ($3::uuid IS NOT NULL AND oauth_grant=$3))
             AND last_seen_at>=clock_timestamp()-interval '30 minutes' ORDER BY last_seen_at DESC,id DESC LIMIT 1)
            RETURNING id
        ), created AS (
            INSERT INTO agent_sessions(token_id,workspace_id,oauth_grant,client,started_at,last_seen_at)
            SELECT $1,$2,$3,$4,clock_timestamp(),clock_timestamp() WHERE NOT EXISTS(SELECT 1 FROM reused) RETURNING id
        ) SELECT id AS "id!" FROM reused UNION ALL SELECT id FROM created"#,principal.token_id,principal.workspace_id,agent.oauth_grant,agent.name).fetch_one(&mut **tx).await.map_err(DbError::from)?;
        Ok(session)
    }
    async fn summaries(
        &self,
        id: Option<Uuid>,
        query: &str,
        before: Option<Uuid>,
        errors_only: bool,
    ) -> Result<Vec<Summary>, Error> {
        if query.len() > 1024 {
            return Err(Error::Invalid);
        }
        let patterns = super::patterns::sql(query);
        sqlx::query_file_as!(
            Summary,
            "src/components/agent_sessions/summary.sql",
            id,
            &patterns,
            before,
            errors_only
        )
        .fetch_all(&self.db.0)
        .await
        .map_err(DbError::from)
        .map_err(Error::from)
    }
    pub async fn list(&self, query: &str, before: Option<Uuid>) -> Result<Page, Error> {
        self.list_filtered(query, before, false).await
    }
    pub async fn list_filtered(
        &self,
        query: &str,
        before: Option<Uuid>,
        errors_only: bool,
    ) -> Result<Page, Error> {
        let mut sessions = self.summaries(None, query, before, errors_only).await?;
        let (all_matches, error_matches) = match sessions.first() {
            Some(session) => (session.total_matches, session.error_matches),
            None if before.is_some() || errors_only => self
                .summaries(None, query, None, false)
                .await?
                .first()
                .map_or((0, 0), |session| {
                    (session.total_matches, session.error_matches)
                }),
            None => (0, 0),
        };
        let total_matches = if errors_only {
            error_matches
        } else {
            all_matches
        };
        let next_cursor = if sessions.len() > 50 {
            sessions.truncate(50);
            sessions.last().map(|s| s.id)
        } else {
            None
        };
        Ok(Page {
            sessions,
            next_cursor,
            total_matches,
            all_matches,
            error_matches,
        })
    }
    pub async fn detail(&self, id: Uuid, before: Option<Uuid>) -> Result<Detail, Error> {
        let session = self
            .summaries(Some(id), "", None, false)
            .await?
            .pop()
            .ok_or(Error::NotFound)?;
        let mut ids=sqlx::query_scalar!("SELECT id FROM executions WHERE session_id=$1 AND ($2::uuid IS NULL OR (started_at,id)<(SELECT started_at,id FROM executions WHERE id=$2 AND session_id=$1)) ORDER BY started_at DESC,id DESC LIMIT 21",id,before).fetch_all(&self.db.0).await.map_err(DbError::from)?;
        let next_cursor = if ids.len() > 20 {
            ids.truncate(20);
            ids.last().copied()
        } else {
            None
        };
        let executions = log::Log::new(self.db.clone())
            .details(&ids)
            .await
            .map_err(Error::Database)?;
        Ok(Detail {
            session,
            executions,
            next_cursor,
        })
    }
    pub async fn activity(&self, before: Option<Uuid>) -> Result<ActivityPage, Error> {
        let mut executions = sqlx::query_as!(Activity, r#"SELECT e.id,e.session_id AS "session_id!",e.status,
            to_char(e.started_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS "started_at!"
            FROM executions e JOIN agent_sessions s ON s.id=e.session_id
            WHERE s.expires_at>now() AND ($1::uuid IS NULL OR (e.started_at,e.id)<(SELECT started_at,id FROM executions WHERE id=$1))
            ORDER BY e.started_at DESC,e.id DESC LIMIT 1001"#, before)
            .fetch_all(&self.db.0).await.map_err(DbError::from)?;
        let next_cursor = if executions.len() > 1000 {
            executions.truncate(1000);
            executions.last().map(|e| e.id)
        } else {
            None
        };
        Ok(ActivityPage {
            executions,
            next_cursor,
        })
    }
    pub async fn rename(&self, id: Uuid, title: &str) -> Result<(), Error> {
        let title = title.trim();
        if title.is_empty() || title.chars().count() > 160 || title.chars().any(char::is_control) {
            return Err(Error::Invalid);
        }
        if sqlx::query!(
            "UPDATE agent_sessions SET title=$2 WHERE id=$1 AND expires_at>now()",
            id,
            title
        )
        .execute(&self.db.0)
        .await
        .map_err(DbError::from)?
        .rows_affected()
            != 1
        {
            return Err(Error::NotFound);
        }
        Ok(())
    }
    pub async fn sweep(&self) -> Result<u64, Error> {
        Ok(
            sqlx::query!("DELETE FROM agent_sessions WHERE expires_at<=now()")
                .execute(&self.db.0)
                .await
                .map_err(DbError::from)?
                .rows_affected(),
        )
    }
}
