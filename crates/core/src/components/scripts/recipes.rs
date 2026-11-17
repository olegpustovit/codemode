use super::{Error, Scripts};
use crate::infrastructure::postgres::Error as DbError;
use serde_json::Value;
use uuid::Uuid;

impl Scripts {
    pub(super) async fn install_recipes(&self, workspace: Uuid) -> Result<(), Error> {
        let sources = sqlx::query!(r#"SELECT s.id,COALESCE(s.python_name,s.slug) AS "alias!",s.config->>'url' AS url,
            jsonb_object_agg(t.upstream_name,jsonb_build_object('input',t.input_schema,'output',t.output_schema)) AS "schemas!"
            FROM sources s JOIN endpoints e ON e.id=$1 AND e.tenant_id=s.tenant_id
            JOIN tools t ON t.source_id=s.id AND (e.all_tools OR EXISTS(SELECT 1 FROM endpoint_tools scope WHERE scope.endpoint_id=e.id AND scope.source_id=s.id AND scope.upstream_name=t.upstream_name))
            WHERE s.config->>'kind'='mcp_http' AND rtrim(s.config->>'url','/') IN ('https://nudge.microapps.space/mcp','https://moneybag.microapps.space/mcp')
            AND (e.all_sources OR EXISTS(SELECT 1 FROM endpoint_sources es WHERE es.endpoint_id=e.id AND es.source_id=s.id))
            AND t.upstream_name IN ('list_teams','list_issue_statuses','create_issue','entries_list')
            GROUP BY s.id ORDER BY s.id"#,workspace).fetch_all(&self.db.0).await.map_err(DbError::from)?;
        let candidates: Vec<_> = sources
            .into_iter()
            .filter_map(|source| {
                recipe(source.url.as_deref()?, &source.schemas)
                    .map(|recipe| (source.id, source.alias, recipe))
            })
            .collect();
        if candidates.is_empty() {
            return Ok(());
        }
        let mut tx = self.db.0.begin().await.map_err(DbError::from)?;
        sqlx::query!("SELECT id FROM endpoints WHERE id=$1 FOR UPDATE", workspace)
            .fetch_optional(&mut *tx)
            .await
            .map_err(DbError::from)?
            .ok_or(Error::NotFound)?;
        let mut count = sqlx::query_scalar!(
            "SELECT count(*) AS \"count!\" FROM saved_scripts WHERE endpoint_id=$1",
            workspace
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(DbError::from)?;
        for (source, alias, recipe) in candidates {
            if count >= 128 {
                break;
            }
            let installed = sqlx::query!("INSERT INTO source_recipe_installs(workspace_id,source_id,recipe) VALUES($1,$2,$3) ON CONFLICT DO NOTHING RETURNING recipe",workspace,source,recipe.key).fetch_optional(&mut *tx).await.map_err(DbError::from)?;
            if installed.is_none() {
                continue;
            }
            let name = format!("{alias}: {}", recipe.name);
            let code = recipe.code.replace(
                "$SOURCE",
                &super::super::catalogue::stubs::identifier(&alias),
            );
            count += sqlx::query!("INSERT INTO saved_scripts(endpoint_id,name,description,code) VALUES($1,$2,$3,$4) ON CONFLICT(endpoint_id,name) DO NOTHING",workspace,name,recipe.description,code).execute(&mut *tx).await.map_err(DbError::from)?.rows_affected() as i64;
        }
        tx.commit().await.map_err(DbError::from)?;
        Ok(())
    }
}

struct Recipe {
    key: &'static str,
    name: &'static str,
    description: &'static str,
    code: &'static str,
}
fn paged(schemas: &Value, name: &str) -> bool {
    schemas.get(name).is_some_and(|schema| {
        super::super::catalogue::stubs::pagination(&schema["input"], schema.get("output")).is_some()
    })
}
fn recipe(url: &str, schemas: &Value) -> Option<Recipe> {
    if url.trim_end_matches('/') == "https://nudge.microapps.space/mcp"
        && paged(schemas, "list_teams")
        && paged(schemas, "list_issue_statuses")
        && schemas["create_issue"]["input"]["properties"]
            .as_object()
            .is_some_and(|properties| {
                ["teamId", "statusId", "title"]
                    .iter()
                    .all(|key| properties.contains_key(*key))
            })
    {
        return Some(Recipe {
            key: "nudge-create-issue",
            name: "Create issue by team and status",
            description: "Creates one Nudge issue after resolving its team and ordinary workflow status. execute(scriptId=id, args={teamKey: str, statusName: str, title: str, description?: str, priority?: int 0..4}).",
            code: include_str!("recipes/nudge_create_issue.py"),
        });
    }
    if url.trim_end_matches('/') == "https://moneybag.microapps.space/mcp"
        && paged(schemas, "entries_list")
        && schemas["entries_list"]["input"]["properties"]
            .as_object()
            .is_some_and(|properties| {
                ["from", "to", "account"]
                    .iter()
                    .all(|key| properties.contains_key(*key))
            })
    {
        return Some(Recipe {
            key: "moneybag-monthly-totals",
            name: "Monthly account totals",
            description: "Exact monthly Moneybag posting totals for an account and its descendants, separated by currency. execute(scriptId=id, args={from: YYYY-MM-DD, to: YYYY-MM-DD, account: str, status?: str, tag?: str, max_pages?: int}).",
            code: include_str!("recipes/moneybag_monthly_totals.py"),
        });
    }
    None
}
