use crate::infrastructure::postgres::{Database, Error};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use uuid::Uuid;
pub mod oauth;
pub mod rate_limit;

#[derive(Clone)]
pub struct Auth {
    db: Database,
}

#[derive(Clone, Debug)]
pub struct Principal {
    pub token_id: Uuid,
    pub tenant_id: Uuid,
    pub workspace_id: Uuid,
    pub resource: String,
    pub audience: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Token {
    pub id: Uuid,
    pub name: String,
    pub audience: String,
    pub workspace_id: Option<Uuid>,
    pub resource: String,
    pub revoked: bool,
    pub oauth: bool,
    pub last_used_at: Option<String>,
}

impl Auth {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub async fn create(&self, name: &str, audience: &str) -> Result<(Uuid, String), Error> {
        sqlx::query!("INSERT INTO endpoints(slug,hostname,all_sources) VALUES('All sources',$1,true) ON CONFLICT(tenant_id) WHERE all_sources DO NOTHING",audience).execute(&self.db.0).await?;
        let workspace=sqlx::query_scalar!("SELECT id FROM endpoints WHERE all_sources AND tenant_id=COALESCE(NULLIF(current_setting('codemode.tenant_id',true),''),'00000000-0000-0000-0000-000000000000')::uuid").fetch_one(&self.db.0).await?;
        self.create_for_workspace(name, workspace, audience, "/mcp")
            .await?
            .ok_or_else(|| Error::from(sqlx::Error::RowNotFound))
    }

    pub async fn create_for_workspace(
        &self,
        name: &str,
        workspace: Uuid,
        audience: &str,
        resource: &str,
    ) -> Result<Option<(Uuid, String)>, Error> {
        let token = format!("cm_{}", URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>()));
        let hash = Sha256::digest(token.as_bytes()).to_vec();
        let id=sqlx::query_scalar!("WITH workspace AS (SELECT id FROM endpoints WHERE id=$1 FOR KEY SHARE) INSERT INTO access_tokens(name,audience,token_hash,workspace_id,resource) SELECT $2,$3,$4,id,$5 FROM workspace RETURNING id",workspace,name,audience,hash,resource).fetch_optional(&self.db.0).await?;
        Ok(id.map(|id| (id, token)))
    }

    pub async fn authenticate(
        &self,
        token: &str,
        audience: &str,
    ) -> Result<Option<Principal>, Error> {
        if !((token.starts_with("cm_") && token.len() == 46)
            || (token.starts_with("cmo_") && token.len() == 47))
        {
            return Ok(None);
        }
        let hash = Sha256::digest(token.as_bytes()).to_vec();
        let row = sqlx::query!("SELECT id,token_hash,audience,tenant_id,workspace_id AS \"workspace_id!\",resource FROM access_tokens WHERE workspace_id IS NOT NULL AND token_hash=$1 AND revoked_at IS NULL AND (expires_at IS NULL OR expires_at>now())",hash)
            .fetch_optional(&self.db.0).await?;
        let Some(row) = row else {
            return Ok(None);
        };
        if !bool::from(hash.ct_eq(&row.token_hash)) || row.audience != audience {
            return Ok(None);
        }
        sqlx::query!(
            "UPDATE access_tokens SET last_used_at=now() WHERE id=$1",
            row.id
        )
        .execute(&self.db.0)
        .await?;
        Ok(Some(Principal {
            token_id: row.id,
            tenant_id: row.tenant_id,
            workspace_id: row.workspace_id,
            resource: row.resource,
            audience: row.audience,
        }))
    }

    pub async fn list(&self) -> Result<Vec<Token>, Error> {
        let rows = sqlx::query!(r#"SELECT id,name,audience,workspace_id,resource,revoked_at IS NOT NULL AS "revoked!",oauth_grant IS NOT NULL AS "oauth!",
            to_char(last_used_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS last_used_at
            FROM access_tokens ORDER BY created_at"#).fetch_all(&self.db.0).await?;
        Ok(rows
            .into_iter()
            .map(|r| Token {
                id: r.id,
                name: r.name,
                audience: r.audience,
                workspace_id: r.workspace_id,
                resource: r.resource,
                revoked: r.revoked,
                oauth: r.oauth,
                last_used_at: r.last_used_at,
            })
            .collect())
    }

    pub async fn revoke(&self, id: Uuid) -> Result<bool, Error> {
        oauth::revoke_access(&self.db, id).await
    }
}
