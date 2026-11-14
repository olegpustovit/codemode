use crate::infrastructure::postgres::{Database, Error};
use uuid::Uuid;

#[derive(Clone)]
pub struct Log {
    db: Database,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub id: Uuid,
    pub token_id: Uuid,
    pub status: String,
    pub error: Option<String>,
    pub tool_calls: i32,
    pub started_at: String,
    pub finished_at: Option<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Call {
    pub seq: i32,
    pub path: String,
    pub args_redacted: serde_json::Value,
    pub outcome: String,
    pub error: Option<String>,
    pub started_at: String,
    pub duration_ms: i64,
}

#[derive(serde::Serialize)]
pub struct Detail {
    #[serde(flatten)]
    pub summary: Summary,
    pub code: String,
    pub calls: Vec<Call>,
}

impl Log {
    pub async fn start_administration(
        &self,
        token: Uuid,
        tool: &str,
        args: serde_json::Value,
    ) -> Result<Uuid, Error> {
        Ok(sqlx::query_scalar!("INSERT INTO administration_calls(token_id,tool,args_redacted,outcome) VALUES($1,$2,$3,'running') RETURNING id",token,tool,args).fetch_one(&self.db.0).await?)
    }
    pub async fn finish_administration(&self, id: Uuid, success: bool) -> Result<(), Error> {
        sqlx::query!(
            "UPDATE administration_calls SET outcome=$2,finished_at=now() WHERE id=$1",
            id,
            if success { "completed" } else { "failed" }
        )
        .execute(&self.db.0)
        .await?;
        Ok(())
    }

    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub async fn list(&self, before: Option<Uuid>) -> Result<Vec<Summary>, Error> {
        Ok(sqlx::query_as!(Summary,r#"SELECT id,token_id,status,error,tool_calls,
            to_char(started_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS "started_at!",
            to_char(finished_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS finished_at
            FROM executions WHERE $1::uuid IS NULL OR (started_at,id)<(SELECT started_at,id FROM executions WHERE id=$1)
            ORDER BY started_at DESC,id DESC LIMIT 50"#,before).fetch_all(&self.db.0).await?)
    }

    pub async fn detail(&self, id: Uuid) -> Result<Option<Detail>, Error> {
        let mut tx = self.db.0.begin().await?;
        sqlx::query!("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
            .execute(&mut *tx)
            .await?;
        let row = sqlx::query!(
            r#"SELECT id,token_id,status,error,tool_calls,code,
            to_char(started_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS "started_at!",
            to_char(finished_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS finished_at
            FROM executions WHERE id=$1"#,
            id
        )
        .fetch_optional(&mut *tx)
        .await?;
        let Some(row) = row else { return Ok(None) };
        let calls=sqlx::query_as!(Call,r#"SELECT seq,path,args_redacted,outcome,error,
            to_char(started_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS "started_at!",duration_ms
            FROM tool_calls WHERE execution_id=$1 ORDER BY seq LIMIT 100"#,id).fetch_all(&mut *tx).await?;
        tx.commit().await?;
        Ok(Some(Detail {
            summary: Summary {
                id: row.id,
                token_id: row.token_id,
                status: row.status,
                error: row.error,
                tool_calls: row.tool_calls,
                started_at: row.started_at,
                finished_at: row.finished_at,
            },
            code: row.code,
            calls,
        }))
    }
}
