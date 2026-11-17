pub mod stubs;
use crate::infrastructure::postgres::{Database, Error};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Clone)]
pub struct Catalogue {
    db: Database,
}

#[derive(Clone, Serialize)]
pub struct Definition {
    pub upstream_name: String,
    pub path: String,
    pub description: String,
    pub input_schema: Value,
    pub output_schema: Option<Value>,
    pub read_only: bool,
    pub destructive: bool,
}

struct RankedMatch {
    path: String,
    description: String,
    signature: String,
    rank: f32,
    blocked: bool,
}

#[derive(Serialize)]
pub struct Match {
    pub blocked: bool,
    pub path: String,
    pub description: String,
    pub signature: String,
}

#[derive(Serialize)]
pub struct Page {
    pub tools: Vec<Match>,
    #[serde(rename = "nextCursor")]
    pub cursor: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct Cursor {
    query: String,
    full: bool,
    rank: f32,
    path: String,
}

#[derive(Debug, thiserror::Error)]
pub enum SearchError {
    #[error("invalid search cursor or query (maximum 1024 bytes)")]
    Invalid,
    #[error(transparent)]
    Database(#[from] Error),
}

impl Catalogue {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub async fn replace(
        &self,
        source_id: Uuid,
        definitions: &[Definition],
        revision: i64,
    ) -> Result<bool, Error> {
        let mut tx = self.db.0.begin().await?;
        if sqlx::query!("UPDATE sources SET status='ready',last_error=NULL,synced_at=now() WHERE id=$1 AND revision=$2",source_id,revision).execute(&mut *tx).await?.rows_affected()==0 {return Ok(false);}
        sqlx::query!("DELETE FROM tools WHERE source_id=$1", source_id)
            .execute(&mut *tx)
            .await?;
        for tool in definitions {
            let stub = stubs::generate(
                &tool.path,
                &tool.description,
                &tool.input_schema,
                tool.output_schema.as_ref(),
            );
            let short = tool
                .path
                .split_once('.')
                .map_or(tool.path.as_str(), |(_, name)| name);
            let names = format!("{} {} {short}", tool.path, tool.upstream_name);
            let words = format!("{names} {}", stubs::search_words(&names));
            sqlx::query!("INSERT INTO tools(source_id,upstream_name,path,description,input_schema,output_schema,stub_brief,stub_full,read_only,destructive,search) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,setweight(to_tsvector('simple',$11),'A') || setweight(to_tsvector('simple',$4),'B'))",
                source_id,tool.upstream_name,tool.path,tool.description,tool.input_schema,tool.output_schema,stub.brief,stub.full,tool.read_only,tool.destructive,words).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(true)
    }

    pub async fn search(
        &self,
        query: &str,
        full: bool,
        cursor: Option<&str>,
        allowed: Option<&[Uuid]>,
    ) -> Result<Page, SearchError> {
        self.search_scoped(query, full, cursor, allowed, None).await
    }
    pub async fn search_scoped(
        &self,
        query: &str,
        full: bool,
        cursor: Option<&str>,
        allowed: Option<&[Uuid]>,
        workspace: Option<Uuid>,
    ) -> Result<Page, SearchError> {
        if query.len() > 1024 {
            return Err(SearchError::Invalid);
        }
        let cursor: Option<Cursor> = cursor
            .map(|text| {
                if text.len() > 4096 {
                    return Err(SearchError::Invalid);
                }
                let bytes = URL_SAFE_NO_PAD
                    .decode(text)
                    .map_err(|_| SearchError::Invalid)?;
                let cursor: Cursor =
                    serde_json::from_slice(&bytes).map_err(|_| SearchError::Invalid)?;
                if cursor.query != query || cursor.full != full || !cursor.rank.is_finite() {
                    return Err(SearchError::Invalid);
                }
                Ok(cursor)
            })
            .transpose()?;
        let rank = cursor.as_ref().map(|c| c.rank);
        let path = cursor.as_ref().map(|c| c.path.as_str());
        let rows=sqlx::query_as!(RankedMatch, r#"WITH ranked AS (
          SELECT path,description,stub_brief,stub_full,
          EXISTS(SELECT 1 FROM tool_policies p WHERE p.endpoint_id=$6 AND p.source_id=t.source_id AND p.upstream_name=t.upstream_name AND p.action='block') AS blocked,
          CASE WHEN btrim($1)='' THEN 0::real ELSE ts_rank(search,websearch_to_tsquery('simple',$1)) END AS rank
          FROM tools t WHERE ($5::uuid[] IS NULL OR source_id=ANY($5)) AND (btrim($1)='' OR search @@ websearch_to_tsquery('simple',$1))
          AND ($6::uuid IS NULL OR NOT EXISTS(SELECT 1 FROM endpoints e WHERE e.id=$6 AND NOT e.all_tools AND NOT EXISTS(SELECT 1 FROM endpoint_tools scope WHERE scope.endpoint_id=e.id AND scope.source_id=t.source_id AND scope.upstream_name=t.upstream_name)))
        ), paged AS (
          SELECT * FROM ranked WHERE $3::real IS NULL OR rank < $3 OR (rank=$3 AND path>$4)
          ORDER BY rank DESC,path LIMIT 11
        ) SELECT path,description,
          CASE WHEN $2 AND row_number() OVER(ORDER BY rank DESC,path)<=3 THEN stub_full ELSE stub_brief END AS "signature!",
          rank AS "rank!",blocked AS "blocked!" FROM paged ORDER BY rank DESC,path"#,query,full,rank,path,allowed,workspace).fetch_all(&self.db.0).await.map_err(Error::from)?;
        page(rows, query, full)
    }
}

// Budget: five allocations for a brief paginated 11-row response, excluding the reused output buffer.
fn page(rows: Vec<RankedMatch>, query: &str, full: bool) -> Result<Page, SearchError> {
    let next = if rows.len() > 10 {
        let row = &rows[9];
        Some(
            URL_SAFE_NO_PAD.encode(
                serde_json::to_vec(&Cursor {
                    query: query.into(),
                    full,
                    rank: row.rank,
                    path: row.path.clone(),
                })
                .map_err(|_| SearchError::Invalid)?,
            ),
        )
    } else {
        None
    };
    let mut definitions = std::collections::BTreeSet::new();
    let tools = rows
        .into_iter()
        .take(10)
        .map(|mut r| {
            if full && r.signature.contains("from typing import") {
                r.signature = r
                    .signature
                    .lines()
                    .filter(|line| {
                        (!line.starts_with("from typing import") && !line.contains(" = TypedDict("))
                            || definitions.insert((*line).to_owned())
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
            }
            Match {
                path: r.path,
                description: first_line(r.description),
                signature: r.signature,
                blocked: r.blocked,
            }
        })
        .collect();
    Ok(Page {
        tools,
        cursor: next,
    })
}

fn first_line(mut text: String) -> String {
    if let Some(end) = text.find(['\n', '\r']) {
        text.truncate(end);
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ranked_rows_to_json_has_a_fixed_allocation_budget() {
        let rows = (0..11)
            .map(|i| RankedMatch {
                path: format!("example.tool_{i}"),
                description: "First line.\nFull documentation.".into(),
                signature: "async def tools.example.tool(*, text: str) -> str".into(),
                rank: 0.5,
                blocked: false,
            })
            .collect();
        let mut output = Vec::with_capacity(32768);
        let allocations = allocation_counter::measure(|| {
            let result = page(rows, "tool", false).unwrap();
            serde_json::to_writer(&mut output, &result).unwrap();
        });
        assert!(
            allocations.count_total <= 5,
            "{} allocations exceed the measured five-allocation page budget",
            allocations.count_total
        );
        let response: serde_json::Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(response["tools"].as_array().unwrap().len(), 10);
        assert_eq!(response["tools"][0]["description"], "First line.");
        assert!(response["nextCursor"].is_string());
    }
}
