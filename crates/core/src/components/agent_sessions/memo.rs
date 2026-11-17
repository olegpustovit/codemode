use super::{AgentSessions, Error};
use crate::infrastructure::postgres::Error as DbError;
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "lowercase", deny_unknown_fields)]
pub enum Request {
    Get { key: String },
    Set { key: String, value: Value },
}

impl AgentSessions {
    pub async fn memo(
        &self,
        execution: Uuid,
        workspace: Uuid,
        request: Request,
    ) -> Result<Value, Error> {
        let key = match &request {
            Request::Get { key } | Request::Set { key, .. } => key,
        };
        if key.is_empty()
            || key.len() > 128
            || key.chars().any(char::is_control)
            || super::super::broker::secret_key(key)
            || credential_string(key)
        {
            return Err(Error::MemoSensitive);
        }
        if let Request::Set { value, .. } = &request {
            if !non_sensitive(value, 0) {
                return Err(Error::MemoSensitive);
            }
            if serde_json::to_vec(value)
                .map_err(|_| Error::MemoLimit)?
                .len()
                > 65536
            {
                return Err(Error::MemoLimit);
            }
        }
        let owner = sqlx::query!(r#"SELECT s.id,a.tenant_id FROM executions e
            JOIN agent_sessions s ON s.id=e.session_id JOIN access_tokens a ON a.id=e.token_id
            WHERE e.id=$1 AND s.workspace_id=$2 AND a.workspace_id=s.workspace_id AND a.resource='/mcp'
            AND s.expires_at>clock_timestamp() AND a.revoked_at IS NULL AND (a.expires_at IS NULL OR a.expires_at>clock_timestamp())"#, execution,workspace)
            .fetch_optional(&self.db.0).await.map_err(DbError::from)?.ok_or(Error::NotFound)?;
        match request {
            Request::Get { key } => {
                let data = sqlx::query!(r#"SELECT data ? $2 AS "found!",data->$2 AS value FROM agent_session_memo WHERE session_id=$1 AND tenant_id=$3"#,owner.id,key,owner.tenant_id)
                    .fetch_optional(&self.db.0).await.map_err(DbError::from)?;
                Ok(match data {
                    Some(row) => serde_json::json!({"found":row.found,"value":row.value}),
                    None => serde_json::json!({"found":false,"value":null}),
                })
            }
            Request::Set { key, value } => {
                let changed = sqlx::query_scalar!(r#"INSERT INTO agent_session_memo(session_id,tenant_id,data)
                    SELECT $1,$2,jsonb_build_object($3::text,$4::jsonb)
                    WHERE octet_length(jsonb_build_object($3::text,$4::jsonb)::text)<=65536
                    ON CONFLICT(session_id) DO UPDATE SET data=agent_session_memo.data || EXCLUDED.data
                    WHERE agent_session_memo.tenant_id=EXCLUDED.tenant_id AND octet_length((agent_session_memo.data || EXCLUDED.data)::text)<=65536
                    RETURNING session_id"#,owner.id,owner.tenant_id,key,value)
                    .fetch_optional(&self.db.0).await.map_err(DbError::from)?;
                if changed.is_none() {
                    return Err(Error::MemoLimit);
                }
                Ok(value)
            }
        }
    }
}

fn credential_string(value: &str) -> bool {
    let value = value.trim();
    [
        "Bearer ",
        "Basic ",
        "cm_",
        "ghp_",
        "gho_",
        "github_pat_",
        "sk-",
        "-----BEGIN ",
    ]
    .iter()
    .any(|prefix| value.starts_with(prefix))
}
fn non_sensitive(value: &Value, depth: usize) -> bool {
    if depth > 32 {
        return false;
    }
    match value {
        Value::String(value) => !credential_string(value),
        Value::Array(values) => values.iter().all(|value| non_sensitive(value, depth + 1)),
        Value::Object(values) => values.iter().all(|(key, value)| {
            !super::super::broker::secret_key(key)
                && !credential_string(key)
                && non_sensitive(value, depth + 1)
        }),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cached_ids_are_allowed_and_credential_keys_and_values_are_rejected() {
        assert!(non_sensitive(
            &serde_json::json!({"teamId":"EVL","statuses":[{"id":"42","name":"Open"}]}),
            0
        ));
        for value in [
            serde_json::json!({"nested":[{"access_token":"opaque"}]}),
            serde_json::json!("Bearer opaque"),
            serde_json::json!({"data":"github_pat_opaque"}),
        ] {
            assert!(!non_sensitive(&value, 0));
        }
    }
}
