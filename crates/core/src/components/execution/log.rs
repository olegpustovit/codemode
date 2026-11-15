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
    pub session_id: Option<Uuid>,
    pub title: Option<String>,
    pub status: String,
    pub error: Option<String>,
    pub tool_calls: i32,
    pub started_at: String,
    pub finished_at: Option<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Call {
    #[serde(skip_serializing)]
    execution_id: Uuid,
    pub seq: i32,
    pub path: String,
    pub args_redacted: serde_json::Value,
    pub outcome: String,
    pub error: Option<String>,
    pub started_at: String,
    pub duration_ms: i64,
    pub preview: Option<String>,
}

#[derive(serde::Serialize)]
pub struct Detail {
    #[serde(flatten)]
    pub summary: Summary,
    pub code: String,
    pub result: Option<String>,
    pub output: Option<String>,
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
        Ok(sqlx::query_as!(Summary,r#"SELECT id,token_id,session_id,title,status,error,tool_calls,
            to_char(started_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS "started_at!",
            to_char(finished_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS finished_at
            FROM executions WHERE (session_id IS NULL OR EXISTS(SELECT 1 FROM agent_sessions s WHERE s.id=session_id AND s.expires_at>now())) AND ($1::uuid IS NULL OR (started_at,id)<(SELECT started_at,id FROM executions WHERE id=$1))
            ORDER BY started_at DESC,id DESC LIMIT 50"#,before).fetch_all(&self.db.0).await?)
    }

    pub async fn detail(&self, id: Uuid) -> Result<Option<Detail>, Error> {
        Ok(self.details(&[id]).await?.pop())
    }
    pub(crate) async fn details(&self, ids: &[Uuid]) -> Result<Vec<Detail>, Error> {
        let mut tx = self.db.0.begin().await?;
        sqlx::query!("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
            .execute(&mut *tx)
            .await?;
        let rows=sqlx::query!(r#"SELECT id,token_id,session_id,title,status,error,tool_calls,code,result,output,
            to_char(started_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS "started_at!",
            to_char(finished_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS finished_at
            FROM executions WHERE id=ANY($1) AND (session_id IS NULL OR EXISTS(SELECT 1 FROM agent_sessions s WHERE s.id=session_id AND s.expires_at>now())) ORDER BY started_at DESC,id DESC"#,ids).fetch_all(&mut *tx).await?;
        let calls=sqlx::query_as!(Call,r#"SELECT execution_id,seq,path,args_redacted,outcome,error,preview,
            to_char(started_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS "started_at!",duration_ms
            FROM tool_calls WHERE execution_id=ANY($1) AND seq BETWEEN 0 AND 99 ORDER BY execution_id,seq"#,ids).fetch_all(&mut *tx).await?;
        tx.commit().await?;
        let mut grouped = std::collections::BTreeMap::<Uuid, Vec<Call>>::new();
        for call in calls {
            grouped.entry(call.execution_id).or_default().push(call);
        }
        Ok(rows
            .into_iter()
            .map(|row| Detail {
                summary: Summary {
                    id: row.id,
                    token_id: row.token_id,
                    session_id: row.session_id,
                    title: row.title,
                    status: row.status,
                    error: row.error,
                    tool_calls: row.tool_calls,
                    started_at: row.started_at,
                    finished_at: row.finished_at,
                },
                code: row.code,
                result: row.result,
                output: row.output,
                calls: grouped.remove(&row.id).unwrap_or_default(),
            })
            .collect())
    }
}

pub(crate) fn preview(text: &str, budget: usize) -> String {
    if text.len() <= budget {
        return text.to_owned();
    }
    let suffix = if budget >= 3 { "…" } else { "" };
    let mut end = budget.saturating_sub(suffix.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{suffix}", &text[..end])
}
#[cfg(test)]
mod tests {
    #[test]
    fn previews_obey_the_byte_budget_without_splitting_utf8() {
        for budget in [0, 1, 2, 3, 4, 5, 2048, 16384] {
            let text = "雪".repeat(6000);
            let preview = super::preview(&text, budget);
            assert!(preview.len() <= budget);
            assert!(preview.is_char_boundary(preview.len()));
        }
        assert_eq!(super::preview("short", 2048), "short");
    }
}
