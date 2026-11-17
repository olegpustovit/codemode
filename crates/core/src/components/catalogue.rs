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

struct ManifestTool {
    source: String,
    path: Option<String>,
    brief: Option<String>,
    output_schema: Option<Value>,
    total: i64,
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

    pub async fn manifest(&self, workspace: Uuid) -> Result<String, Error> {
        let rows = sqlx::query_as!(ManifestTool, r#"WITH visible AS (
            SELECT COALESCE(s.python_name,s.slug) AS source,t.path,t.stub_brief,t.output_schema,
            count(t.path) OVER(PARTITION BY s.id) AS total,
            row_number() OVER(PARTITION BY s.id ORDER BY CASE
                WHEN t.upstream_name IN ('artifact_list','entries_list','list_issues') THEN 0
                WHEN t.upstream_name IN ('artifact_read','doc_create','list_teams','accounts_list','get_issue') THEN 1
                WHEN t.upstream_name IN ('list_issue_statuses','entry_post','tags_set','create_issue') THEN 2
                ELSE 3 END,t.path) AS position,
            EXISTS(SELECT 1 FROM tool_policies p WHERE p.endpoint_id=e.id AND p.source_id=s.id AND p.upstream_name=t.upstream_name AND p.action='block') AS blocked
            FROM sources s JOIN endpoints e ON e.id=$1 AND e.tenant_id=s.tenant_id
            LEFT JOIN tools t ON t.source_id=s.id AND (e.all_tools OR EXISTS(SELECT 1 FROM endpoint_tools scope WHERE scope.endpoint_id=e.id AND scope.source_id=s.id AND scope.upstream_name=t.upstream_name))
            WHERE e.all_sources OR EXISTS(SELECT 1 FROM endpoint_sources es WHERE es.endpoint_id=e.id AND es.source_id=s.id)
        ) SELECT source AS "source!",path AS "path?",stub_brief AS "brief?",output_schema AS "output_schema?",total AS "total!",blocked AS "blocked!" FROM visible WHERE position<=5 ORDER BY source,position"#, workspace)
            .fetch_all(&self.db.0).await?;
        Ok(manifest(rows))
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

fn manifest(rows: Vec<ManifestTool>) -> String {
    let mut groups = std::collections::BTreeMap::<String, (i64, Vec<String>)>::new();
    for row in rows {
        let ManifestTool {
            source,
            path,
            brief,
            output_schema,
            total,
            blocked,
        } = row;
        let source_prefix = format!("async def tools.{source}.");
        let group = groups.entry(source).or_insert_with(|| (total, Vec::new()));
        if let Some(path) = path {
            let name = path
                .rsplit_once('.')
                .map_or(path.as_str(), |(_, name)| name);
            let signature = brief
                .as_deref()
                .and_then(|brief| brief.lines().next())
                .and_then(|line| line.strip_prefix(&source_prefix))
                .and_then(|line| line.strip_suffix(": ..."))
                .filter(|line| line.len() <= 480);
            let shape = output_schema.as_ref().map(stubs::return_skeleton);
            group.1.push(match (signature, shape) {
                _ if blocked => format!("{name} [blocked]"),
                (Some(signature), Some(shape)) => {
                    let suffix = format!(" -> {shape}");
                    if signature.ends_with(&suffix) {
                        signature.to_owned()
                    } else {
                        format!("{signature}{suffix}")
                    }
                }
                (Some(signature), None) => signature.to_owned(),
                (None, _) => format!("{name} (use help)"),
            });
        }
    }
    let count: i64 = groups.values().map(|(count, _)| *count).sum();
    let mut text = format!(
        "Connected sources ({count} tools), already loaded inside execute as tools.<source>.<tool>: "
    );
    if groups.is_empty() {
        text.push_str("none");
    }
    let total = groups.len();
    for (index, (source, (count, mut names))) in groups.into_iter().enumerate() {
        if count > names.len() as i64 {
            names.push("…".into());
        }
        let source = source.chars().take(80).collect::<String>();
        let available = 3072usize.saturating_sub(text.len() + source.len() + 90);
        let mut entries = String::new();
        for name in names {
            if entries.len() + name.len() + 6 > available {
                entries.push_str("…");
                break;
            }
            entries.push_str(&name);
            entries.push_str("; ");
        }
        let entry = format!("{source} ({count}: {})", entries.trim_end_matches("; "));
        if text.len() + entry.len() + 80 > 3072 {
            text.push_str(&format!(
                "… {} more sources; discover with tools.search",
                total - index
            ));
            break;
        }
        if index > 0 {
            text.push_str("; ");
        }
        text.push_str(&entry);
    }
    text
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
    if text.len() > 180 {
        let mut end = 177;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str("...");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn discovery_descriptions_are_compact_utf8_without_extra_allocations() {
        assert_eq!(first_line("First line.\nMore docs.".into()), "First line.");
        assert_eq!(first_line("x".repeat(180)).len(), 180);
        let long = "界".repeat(100);
        let mut compact = String::new();
        let allocations = allocation_counter::measure(|| compact = first_line(long));
        assert_eq!(allocations.count_total, 0);
        assert_eq!(compact.len(), 180);
        assert!(compact.ends_with("..."));
    }
    #[test]
    fn manifests_name_sources_tools_blocks_and_stay_within_the_byte_budget() {
        let text = manifest(vec![
            ManifestTool {
                source: "nudge".into(),
                path: Some("nudge.list_issues".into()),
                brief: Some("async def tools.nudge.list_issues(*, limit: int = None): ...".into()),
                output_schema: Some(
                    serde_json::json!({"properties":{"issues":{"items":{"properties":{"id":{"type":"string"}}}}}}),
                ),
                total: 15,
                blocked: false,
            },
            ManifestTool {
                source: "artifacts".into(),
                path: Some("artifacts.artifact_delete".into()),
                brief: None,
                output_schema: None,
                total: 1,
                blocked: true,
            },
        ]);
        assert!(text.contains("Connected sources (16 tools)"));
        assert!(text.contains("artifact_delete [blocked]"));
        assert!(
            text.contains("nudge (15: list_issues(*, limit: int = None) -> {issues:[{id}]}; …)")
        );
        let rows = (0..64)
            .map(|i| ManifestTool {
                source: format!("source_{i}_{}", "s".repeat(80)),
                path: Some("x".repeat(200)),
                brief: None,
                output_schema: None,
                total: 100,
                blocked: false,
            })
            .collect();
        let bounded = manifest(rows);
        assert!(bounded.len() <= 3072);
        assert!(bounded.contains("more sources"));
        assert!(manifest(vec![]).contains("none"));
    }

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
