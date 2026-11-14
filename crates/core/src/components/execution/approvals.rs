use super::super::{
    auth::Principal,
    broker::{Prepared, redact},
};
use crate::infrastructure::postgres::{Database, Error as DbError};
use hmac::{Hmac, Mac};
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};
use serde_json::{Value, value::RawValue};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, sync::Arc};
use uuid::Uuid;

const MAX_CONTEXT: usize = 9 * 1024 * 1024;
const MAX_SNAPSHOT: usize = 72 * 1024 * 1024;
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("execution is not paused for this caller and hostname")]
    NotFound,
    #[error("approval expired or its caller is no longer authorized")]
    Expired,
    #[error("approval snapshot could not be authenticated")]
    Invalid,
    #[error("approval changed or was already decided; reload before continuing")]
    Conflict,
    #[error("approval capacity is full; retry later")]
    Busy,
    #[error(transparent)]
    Database(#[from] DbError),
}
#[derive(Serialize, Deserialize)]
pub(crate) struct Frozen {
    pub execution_id: Uuid,
    pub pause_id: Uuid,
    pub token_id: Uuid,
    pub audience: String,
    pub expires: i64,
    pub code_hash: [u8; 32],
    pub count: usize,
    pub pending: Vec<Prepared>,
}
pub(crate) enum Claim {
    Pending(usize),
    Ready(Continuation),
}
pub(crate) struct Continuation {
    pub frozen: Frozen,
    pub snapshot: Arc<Vec<u8>>,
    pub decisions: Vec<(u32, bool)>,
}
struct Verified {
    frozen: Frozen,
    snapshot: Arc<Vec<u8>>,
    signature: [u8; 32],
    code: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub execution_id: Uuid,
    pub hostname: String,
    pub expires_at: String,
    pub pending: i64,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Detail {
    pub execution_id: Uuid,
    pub pause_id: Uuid,
    pub hostname: String,
    pub code: String,
    pub expires_at: String,
    pub calls: Vec<Call>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Call {
    pub call_id: u32,
    pub path: String,
    pub args: Box<RawValue>,
    pub error: Option<String>,
    pub decision: Option<bool>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Decision {
    pub call_id: u32,
    pub allow: bool,
}
#[derive(Clone)]
pub struct Approvals {
    db: Database,
    key: Arc<secrecy::SecretBox<[u8; 32]>>,
    crypto: Arc<tokio::sync::Semaphore>,
}
fn tag(key: &[u8; 32], context: &[u8], snapshot: &[u8]) -> Result<[u8; 32], Error> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).map_err(|_| Error::Invalid)?;
    mac.update(b"codemode-approval-v1\0");
    mac.update(&(context.len() as u64).to_le_bytes());
    mac.update(context);
    mac.update(snapshot);
    Ok(mac.finalize().into_bytes().into())
}
impl Approvals {
    pub fn with_database(&self, db: Database) -> Self {
        Self { db, ..self.clone() }
    }

    pub fn new(db: Database, key: [u8; 32]) -> Self {
        Self {
            db,
            key: Arc::new(secrecy::SecretBox::new(Box::new(key))),
            crypto: Arc::new(tokio::sync::Semaphore::new(8)),
        }
    }
    pub(crate) async fn save(&self, frozen: Frozen, snapshot: Vec<u8>) -> Result<(), Error> {
        if snapshot.len() > MAX_SNAPSHOT {
            return Err(Error::Invalid);
        }
        let permit = self
            .crypto
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Busy)?;
        let key = *self.key.expose_secret();
        let (frozen, context, snapshot, signature, audits) =
            tokio::task::spawn_blocking(move || {
                let _permit = permit;
                let context = serde_json::to_vec(&frozen).map_err(|_| Error::Invalid)?;
                if context.len() > MAX_CONTEXT {
                    return Err(Error::Invalid);
                }
                let signature = tag(&key, &context, &snapshot)?;
                let audits = frozen
                    .pending
                    .iter()
                    .map(|pending| {
                        let value: Value = serde_json::from_str(pending.call.args.get())
                            .map_err(|_| Error::Invalid)?;
                        Ok(redact(&value))
                    })
                    .collect::<Result<Vec<_>, Error>>()?;
                Ok((frozen, context, snapshot, signature, audits))
            })
            .await
            .map_err(|_| Error::Invalid)??;
        let mut tx = self.db.0.begin().await.map_err(DbError::from)?;
        sqlx::query!("LOCK TABLE paused_runs IN SHARE ROW EXCLUSIVE MODE")
            .execute(&mut *tx)
            .await
            .map_err(DbError::from)?;
        let count = sqlx::query_scalar!("SELECT count(*) AS \"count!\" FROM paused_runs")
            .fetch_one(&mut *tx)
            .await
            .map_err(DbError::from)?;
        if count >= 64 {
            return Err(Error::Busy);
        }
        let owner=sqlx::query!("SELECT e.id FROM executions e JOIN access_tokens a ON a.id=e.token_id WHERE e.id=$1 AND e.token_id=$2 AND e.status='running' AND a.audience=$3 AND a.revoked_at IS NULL AND (a.expires_at IS NULL OR a.expires_at>now()) FOR UPDATE OF e",frozen.execution_id,frozen.token_id,frozen.audience).fetch_optional(&mut *tx).await.map_err(DbError::from)?;
        if owner.is_none() {
            return Err(Error::Expired);
        }
        sqlx::query!("INSERT INTO paused_runs(execution_id,pause_id,token_id,audience,expires_at,context,snapshot,signature) VALUES($1,$2,$3,$4,to_timestamp($5::bigint::double precision),$6,$7,$8)",frozen.execution_id,frozen.pause_id,frozen.token_id,frozen.audience,frozen.expires,&context,&snapshot,signature.as_slice()).execute(&mut *tx).await.map_err(DbError::from)?;
        let first = frozen
            .count
            .checked_sub(frozen.pending.len())
            .ok_or(Error::Invalid)?;
        for (index, (pending, args)) in frozen.pending.iter().zip(audits).enumerate() {
            let decision = pending.error.as_ref().map(|_| false);
            sqlx::query!("INSERT INTO approval_calls(execution_id,pause_id,call_id,path,args_redacted,error,decision) VALUES($1,$2,$3,$4,$5,$6,$7)",frozen.execution_id,frozen.pause_id,i64::from(pending.call.id),pending.call.path,args,pending.error,decision).execute(&mut *tx).await.map_err(DbError::from)?;
            sqlx::query!("INSERT INTO tool_calls(execution_id,seq,path,args_redacted,outcome) VALUES($1,$2,$3,$4,'awaiting_approval')",frozen.execution_id,(first+index)as i32,pending.call.path,args).execute(&mut *tx).await.map_err(DbError::from)?;
        }
        sqlx::query!("UPDATE executions SET status='paused',tool_calls=$2,error=NULL,finished_at=NULL WHERE id=$1",frozen.execution_id,frozen.count as i32).execute(&mut *tx).await.map_err(DbError::from)?;
        tx.commit().await.map_err(DbError::from)?;
        Ok(())
    }
    async fn inspect(&self, id: Uuid, principal: Option<&Principal>) -> Result<Verified, Error> {
        let token = principal.map(|p| p.token_id);
        let audience = principal.map(|p| p.audience.as_str());
        let row=sqlx::query!(r#"SELECT p.pause_id,p.token_id,p.audience,p.context,p.snapshot,p.signature,
            extract(epoch FROM p.expires_at)::bigint AS "expires!",e.code,
            (p.expires_at>now() AND a.revoked_at IS NULL AND a.audience=p.audience AND (a.expires_at IS NULL OR a.expires_at>now())
             AND EXISTS(SELECT 1 FROM endpoints ep WHERE ep.id=a.workspace_id)
             AND (a.oauth_grant IS NULL OR EXISTS(SELECT 1 FROM oauth_grants g JOIN endpoints ep ON ep.id=g.endpoint_id WHERE g.id=a.oauth_grant AND g.revoked_at IS NULL AND g.expires_at>now() AND ep.id=a.workspace_id))) AS "live!"
            FROM paused_runs p JOIN executions e ON e.id=p.execution_id JOIN access_tokens a ON a.id=p.token_id
            WHERE p.execution_id=$1 AND e.status='paused' AND ($2::uuid IS NULL OR p.token_id=$2) AND ($3::text IS NULL OR p.audience=$3)"#,id,token,audience)
            .fetch_optional(&self.db.0).await.map_err(DbError::from)?;
        let Some(row) = row else {
            let status=sqlx::query_scalar!("SELECT e.status FROM executions e JOIN access_tokens a ON a.id=e.token_id WHERE e.id=$1 AND ($2::uuid IS NULL OR e.token_id=$2) AND ($3::text IS NULL OR a.audience=$3)",id,token,audience).fetch_optional(&self.db.0).await.map_err(DbError::from)?;
            return Err(if status.as_deref() == Some("expired") {
                Error::Expired
            } else {
                Error::NotFound
            });
        };
        if !row.live {
            return Err(Error::Expired);
        }
        let signature: [u8; 32] = row.signature.try_into().map_err(|_| Error::Invalid)?;
        let context = Arc::new(row.context);
        let snapshot = Arc::new(row.snapshot);
        if context.len() > MAX_CONTEXT || snapshot.len() > MAX_SNAPSHOT {
            return Err(Error::Invalid);
        }
        let context_for_job = context.clone();
        let snapshot_for_job = snapshot.clone();
        let key = *self.key.expose_secret();
        let permit = self
            .crypto
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Busy)?;
        let frozen: Frozen = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            use subtle::ConstantTimeEq;
            if !bool::from(tag(&key, &context_for_job, &snapshot_for_job)?.ct_eq(&signature)) {
                return Err(Error::Invalid);
            }
            serde_json::from_slice(&context_for_job).map_err(|_| Error::Invalid)
        })
        .await
        .map_err(|_| Error::Invalid)??;
        if frozen.execution_id != id
            || frozen.pause_id != row.pause_id
            || frozen.token_id != row.token_id
            || frozen.audience != row.audience
            || frozen.expires != row.expires
            || frozen.code_hash != <[u8; 32]>::from(Sha256::digest(row.code.as_bytes()))
            || frozen.count > 100
            || frozen.count < frozen.pending.len()
            || frozen.pending.len() > 100
        {
            return Err(Error::Invalid);
        }
        let mut ids = BTreeSet::new();
        if frozen.pending.iter().any(|p| !ids.insert(p.call.id)) {
            return Err(Error::Invalid);
        }
        Ok(Verified {
            frozen,
            snapshot,
            signature,
            code: row.code,
        })
    }
    pub async fn list(&self) -> Result<Vec<Summary>, Error> {
        Ok(sqlx::query_as!(Summary,r#"SELECT p.execution_id,p.audience AS hostname,
            to_char(p.expires_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS"Z"') AS "expires_at!",
            (SELECT count(*) FROM approval_calls c WHERE c.pause_id=p.pause_id AND c.decision IS NULL) AS "pending!"
            FROM paused_runs p JOIN executions e ON e.id=p.execution_id WHERE e.status='paused' AND p.expires_at>now() ORDER BY e.started_at DESC,p.execution_id LIMIT 64"#).fetch_all(&self.db.0).await.map_err(DbError::from)?)
    }
    pub async fn detail(&self, id: Uuid) -> Result<Detail, Error> {
        let verified = self.inspect(id, None).await?;
        let row=sqlx::query!(r#"SELECT to_char(expires_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS"Z"') AS "expires!" FROM paused_runs WHERE execution_id=$1 AND pause_id=$2"#,id,verified.frozen.pause_id).fetch_optional(&self.db.0).await.map_err(DbError::from)?.ok_or(Error::Conflict)?;
        let states = sqlx::query!(
            "SELECT call_id,decision FROM approval_calls WHERE pause_id=$1",
            verified.frozen.pause_id
        )
        .fetch_all(&self.db.0)
        .await
        .map_err(DbError::from)?;
        let calls = verified
            .frozen
            .pending
            .into_iter()
            .map(|pending| Call {
                call_id: pending.call.id,
                path: pending.call.path,
                args: pending.call.args,
                error: pending.error,
                decision: states
                    .iter()
                    .find(|s| s.call_id == i64::from(pending.call.id))
                    .and_then(|s| s.decision),
            })
            .collect();
        Ok(Detail {
            execution_id: id,
            pause_id: verified.frozen.pause_id,
            hostname: verified.frozen.audience,
            code: verified.code,
            expires_at: row.expires,
            calls,
        })
    }
    pub async fn decide(
        &self,
        id: Uuid,
        pause: Uuid,
        decisions: Vec<Decision>,
    ) -> Result<(), Error> {
        if decisions.is_empty() || decisions.len() > 100 {
            return Err(Error::Invalid);
        }
        let verified = self.inspect(id, None).await?;
        if verified.frozen.pause_id != pause {
            return Err(Error::Conflict);
        }
        let mut unique = BTreeSet::new();
        if decisions.iter().any(|d| {
            !unique.insert(d.call_id)
                || !verified
                    .frozen
                    .pending
                    .iter()
                    .any(|p| p.call.id == d.call_id && p.error.is_none())
        }) {
            return Err(Error::Invalid);
        }
        let mut tx = self.db.0.begin().await.map_err(DbError::from)?;
        sqlx::query!("SELECT execution_id FROM paused_runs WHERE execution_id=$1 AND pause_id=$2 AND signature=$3 AND expires_at>now() FOR UPDATE",id,pause,verified.signature.as_slice()).fetch_optional(&mut *tx).await.map_err(DbError::from)?.ok_or(Error::Conflict)?;
        for decision in decisions {
            if sqlx::query!("UPDATE approval_calls SET decision=$3,decided_at=now() WHERE pause_id=$1 AND call_id=$2 AND decision IS NULL AND error IS NULL",pause,i64::from(decision.call_id),decision.allow).execute(&mut *tx).await.map_err(DbError::from)?.rows_affected()!=1 {return Err(Error::Conflict);}
        }
        tx.commit().await.map_err(DbError::from)?;
        Ok(())
    }
    pub(crate) async fn claim(&self, id: Uuid, principal: &Principal) -> Result<Claim, Error> {
        let verified = self.inspect(id, Some(principal)).await?;
        let mut tx = self.db.0.begin().await.map_err(DbError::from)?;
        sqlx::query!("SELECT p.execution_id FROM paused_runs p JOIN access_tokens a ON a.id=p.token_id WHERE p.execution_id=$1 AND p.pause_id=$2 AND p.signature=$3 AND p.expires_at>now() AND a.revoked_at IS NULL AND a.audience=p.audience AND (a.expires_at IS NULL OR a.expires_at>now()) AND EXISTS(SELECT 1 FROM endpoints ep WHERE ep.id=a.workspace_id) AND (a.oauth_grant IS NULL OR EXISTS(SELECT 1 FROM oauth_grants g JOIN endpoints ep ON ep.id=g.endpoint_id WHERE g.id=a.oauth_grant AND g.revoked_at IS NULL AND g.expires_at>now() AND ep.id=a.workspace_id)) FOR UPDATE OF p",id,verified.frozen.pause_id,verified.signature.as_slice()).fetch_optional(&mut *tx).await.map_err(DbError::from)?.ok_or(Error::Conflict)?;
        let states = sqlx::query!(
            "SELECT call_id,decision FROM approval_calls WHERE pause_id=$1",
            verified.frozen.pause_id
        )
        .fetch_all(&mut *tx)
        .await
        .map_err(DbError::from)?;
        if states.len() != verified.frozen.pending.len() {
            return Err(Error::Invalid);
        }
        if states.iter().any(|s| s.decision.is_none()) {
            return Ok(Claim::Pending(verified.frozen.count));
        }
        let mut decisions = Vec::with_capacity(states.len());
        for pending in &verified.frozen.pending {
            let state = states
                .iter()
                .find(|s| s.call_id == i64::from(pending.call.id))
                .ok_or(Error::Invalid)?;
            decisions.push((pending.call.id, state.decision.ok_or(Error::Invalid)?));
        }
        sqlx::query!(
            "DELETE FROM paused_runs WHERE execution_id=$1 AND pause_id=$2",
            id,
            verified.frozen.pause_id
        )
        .execute(&mut *tx)
        .await
        .map_err(DbError::from)?;
        sqlx::query!(
            "UPDATE executions SET status='running',error=NULL WHERE id=$1",
            id
        )
        .execute(&mut *tx)
        .await
        .map_err(DbError::from)?;
        tx.commit().await.map_err(DbError::from)?;
        Ok(Claim::Ready(Continuation {
            frozen: verified.frozen,
            snapshot: verified.snapshot,
            decisions,
        }))
    }
    pub async fn sweep(&self) -> Result<(), Error> {
        let mut tx = self.db.0.begin().await.map_err(DbError::from)?;
        sqlx::query!("SELECT execution_id FROM paused_runs ORDER BY execution_id FOR UPDATE")
            .fetch_all(&mut *tx)
            .await
            .map_err(DbError::from)?;
        let expired=sqlx::query_scalar!("UPDATE executions e SET status='expired',error='Approval expired or caller was revoked',finished_at=now() WHERE e.status='paused' AND (NOT EXISTS(SELECT 1 FROM paused_runs p WHERE p.execution_id=e.id AND p.expires_at>now()) OR NOT EXISTS(SELECT 1 FROM access_tokens a JOIN endpoints ep ON ep.id=a.workspace_id WHERE a.id=e.token_id AND a.revoked_at IS NULL AND (a.expires_at IS NULL OR a.expires_at>now()) AND (a.oauth_grant IS NULL OR EXISTS(SELECT 1 FROM oauth_grants g WHERE g.id=a.oauth_grant AND g.endpoint_id=ep.id AND g.revoked_at IS NULL AND g.expires_at>now())))) RETURNING e.id").fetch_all(&mut *tx).await.map_err(DbError::from)?;
        sqlx::query!("UPDATE tool_calls SET outcome='expired',error='Approval expired or caller was revoked' WHERE execution_id=ANY($1) AND outcome='awaiting_approval'",&expired).execute(&mut *tx).await.map_err(DbError::from)?;
        sqlx::query!(
            "DELETE FROM paused_runs WHERE execution_id=ANY($1) OR expires_at<=now()",
            &expired
        )
        .execute(&mut *tx)
        .await
        .map_err(DbError::from)?;
        tx.commit().await.map_err(DbError::from)?;
        Ok(())
    }
}
