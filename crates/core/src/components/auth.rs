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

impl Auth {
    pub async fn revoke_many(
        &self,
        selection: super::bulk::Selection<Uuid>,
        expected: i64,
    ) -> Result<u64, super::bulk::Error> {
        use super::bulk::Error as BulkError;
        selection.validate(expected)?;
        let patterns = selection.patterns();
        let mut tx = self.db.0.begin().await.map_err(Error::from)?;
        let selected=sqlx::query!("SELECT id,oauth_grant FROM access_tokens WHERE ($1::uuid[] IS NULL OR id=ANY($1)) AND NOT EXISTS(SELECT 1 FROM unnest($2::text[]) terms(pattern) WHERE NOT(name ILIKE pattern)) ORDER BY id",selection.ids.as_deref(),&patterns).fetch_all(&mut *tx).await.map_err(Error::from)?;
        if selected.len() as i64 != expected {
            return Err(BulkError::Changed);
        }
        let ids: Vec<_> = selected.iter().map(|token| token.id).collect();
        let grants: Vec<_> = selected
            .iter()
            .filter_map(|token| token.oauth_grant)
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        // Match OAuth's endpoint -> grant -> access-token lock order, including refresh siblings.
        sqlx::query!("SELECT e.id FROM endpoints e WHERE EXISTS(SELECT 1 FROM oauth_grants g WHERE g.endpoint_id=e.id AND g.id=ANY($1)) ORDER BY e.id FOR KEY SHARE",&grants).fetch_all(&mut *tx).await.map_err(Error::from)?;
        sqlx::query!(
            "SELECT id FROM oauth_grants WHERE id=ANY($1) ORDER BY id FOR UPDATE",
            &grants
        )
        .fetch_all(&mut *tx)
        .await
        .map_err(Error::from)?;
        let changed=sqlx::query!("SELECT id,workspace_id FROM access_tokens WHERE (id=ANY($1) OR oauth_grant=ANY($2)) AND revoked_at IS NULL ORDER BY id FOR UPDATE",&ids,&grants).fetch_all(&mut *tx).await.map_err(Error::from)?;
        for grant in grants {
            oauth::revoke_grant(&mut tx, grant)
                .await
                .map_err(Error::from)?;
        }
        sqlx::query!(
            "UPDATE access_tokens SET revoked_at=now() WHERE id=ANY($1) AND revoked_at IS NULL",
            &ids
        )
        .execute(&mut *tx)
        .await
        .map_err(Error::from)?;
        let request = Uuid::new_v4();
        for token in &changed {
            sqlx::query!("INSERT INTO control_changes(request_id,workspace_id,kind,item,action) VALUES($1,$2,'token',$3,'revoke')",request,token.workspace_id,token.id.to_string()).execute(&mut *tx).await.map_err(Error::from)?;
        }
        tx.commit().await.map_err(Error::from)?;
        Ok(changed.len() as u64)
    }
}
