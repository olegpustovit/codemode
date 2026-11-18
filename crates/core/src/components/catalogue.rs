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
    observed_shape: Option<String>,
    total: i64,
    blocked: bool,
}

#[derive(Serialize)]
pub struct Match {
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub suggested: bool,
    pub blocked: bool,
    pub path: String,
    pub description: String,
    pub signature: String,
}

#[derive(Serialize)]
pub struct Page {
    pub tools: Vec<Match>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub suggestions: Vec<Match>,
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
            CASE WHEN t.observed_revision=s.revision THEN t.observed_shape END AS observed_shape,
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
        ) SELECT source AS "source!",path AS "path?",stub_brief AS "brief?",output_schema AS "output_schema?",observed_shape,total AS "total!",blocked AS "blocked!" FROM visible WHERE position<=5 ORDER BY source,position"#, workspace)
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
        let observed = sqlx::query!("SELECT upstream_name,output_schema,observed_shape FROM tools WHERE source_id=$1 AND observed_revision=$2 AND observed_shape IS NOT NULL", source_id, revision)
            .fetch_all(&mut *tx).await?.into_iter()
            .map(|row| (row.upstream_name, (row.output_schema, row.observed_shape)))
            .collect::<std::collections::BTreeMap<_, _>>();
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
            let observed_shape = observed
                .get(&tool.upstream_name)
                .filter(|(schema, _)| schema == &tool.output_schema)
                .and_then(|(_, shape)| shape.as_deref());
            sqlx::query!("INSERT INTO tools(source_id,upstream_name,path,description,input_schema,output_schema,stub_brief,stub_full,read_only,destructive,search,observed_shape,observed_revision) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,setweight(to_tsvector('simple',$11),'A') || setweight(to_tsvector('simple',$4),'B'),$12,$13)",
                source_id,tool.upstream_name,tool.path,tool.description,tool.input_schema,tool.output_schema,stub.brief,stub.full,tool.read_only,tool.destructive,words,observed_shape,observed_shape.map(|_| revision)).execute(&mut *tx).await?;
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
        let prefix = prefix_query(query);
        let rows = sqlx::query_as!(RankedMatch, r#"WITH lexemes AS (
          SELECT websearch_to_tsquery('simple',$1) AS exact,
            regexp_replace(websearch_to_tsquery('simple',$7)::text,
              $pattern$'([[:alnum:]]+)'$pattern$, $replacement$'\1':*$replacement$, 'g')::tsquery AS prefix
        ), ranked AS (
          SELECT t.path,t.description,t.stub_brief,t.stub_full,
          CASE WHEN t.observed_revision=s.revision THEN t.observed_shape END AS observed_shape,
          EXISTS(SELECT 1 FROM tool_policies p WHERE p.endpoint_id=$6 AND p.source_id=t.source_id AND p.upstream_name=t.upstream_name AND p.action='block') AS blocked,
          CASE WHEN btrim($1)='' THEN 0::real ELSE (2*(t.path=$1 OR t.upstream_name=$1)::int
              + ts_rank(t.search,lexemes.prefix) + 0.1*ts_rank(t.search,lexemes.exact))::real END AS rank
          FROM tools t JOIN sources s ON s.id=t.source_id CROSS JOIN lexemes
          WHERE ($5::uuid[] IS NULL OR t.source_id=ANY($5))
            AND (btrim($1)='' OR t.search @@ lexemes.exact OR t.search @@ lexemes.prefix)
            AND ($6::uuid IS NULL OR EXISTS(SELECT 1 FROM endpoints e WHERE e.id=$6 AND e.tenant_id=s.tenant_id
              AND (e.all_sources OR EXISTS(SELECT 1 FROM endpoint_sources x WHERE x.endpoint_id=e.id AND x.source_id=s.id))
              AND (e.all_tools OR EXISTS(SELECT 1 FROM endpoint_tools x WHERE x.endpoint_id=e.id AND x.source_id=s.id AND x.upstream_name=t.upstream_name))))
        ), paged AS (
          SELECT * FROM ranked WHERE $3::real IS NULL OR rank < $3 OR (rank=$3 AND path>$4)
          ORDER BY rank DESC,path LIMIT 11
        ) SELECT path,description,
          (CASE WHEN $2 AND row_number() OVER(ORDER BY rank DESC,path)<=3 THEN stub_full ELSE stub_brief END
            || COALESCE(E'\n# Observed result shape: ' || observed_shape,'')) AS "signature!",
          rank AS "rank!",blocked AS "blocked!" FROM paged ORDER BY rank DESC,path"#,
            query,full,rank,path,allowed,workspace,prefix).fetch_all(&self.db.0).await.map_err(Error::from)?;
        let mut page = page(rows, query, full)?;
        if page.tools.is_empty() && cursor.is_none() && !prefix.is_empty() {
            page.suggestions = self.suggestions(query, allowed, workspace).await?;
        }
        Ok(page)
    }

    async fn suggestions(
        &self,
        query: &str,
        allowed: Option<&[Uuid]>,
        workspace: Option<Uuid>,
    ) -> Result<Vec<Match>, SearchError> {
        Ok(sqlx::query_as!(Match, r#"SELECT t.path,t.description,t.stub_brief AS signature,
            true AS "suggested!",false AS "blocked!"
          FROM tools t JOIN sources s ON s.id=t.source_id
          WHERE ($2::uuid[] IS NULL OR t.source_id=ANY($2))
            AND similarity(t.path,$1) >= 0.15
            AND ($3::uuid IS NULL OR EXISTS(SELECT 1 FROM endpoints e WHERE e.id=$3 AND e.tenant_id=s.tenant_id
              AND (e.all_sources OR EXISTS(SELECT 1 FROM endpoint_sources x WHERE x.endpoint_id=e.id AND x.source_id=s.id))
              AND (e.all_tools OR EXISTS(SELECT 1 FROM endpoint_tools x WHERE x.endpoint_id=e.id AND x.source_id=s.id AND x.upstream_name=t.upstream_name))
              AND NOT EXISTS(SELECT 1 FROM tool_policies p WHERE p.endpoint_id=e.id AND p.source_id=s.id AND p.upstream_name=t.upstream_name AND p.action='block')))
          ORDER BY similarity(t.path,$1) DESC,t.path LIMIT 3"#, query, allowed, workspace)
            .fetch_all(&self.db.0).await.map_err(Error::from)?
            .into_iter().map(|mut hit| {hit.description=first_line(hit.description); hit}).collect())
    }

    pub(crate) async fn remember_shape(
        &self,
        source: Uuid,
        revision: i64,
        path: &str,
        shape: &str,
    ) -> Result<(), Error> {
        sqlx::query!("UPDATE tools t SET observed_shape=$4,observed_revision=$2 FROM sources s WHERE s.id=$1 AND s.revision=$2 AND t.source_id=s.id AND t.path=$3 AND t.observed_revision IS DISTINCT FROM $2", source, revision, path, shape)
            .execute(&self.db.0).await?;
        Ok(())
    }
}

fn prefix_query(query: &str) -> String {
    // Preserve exact phrase and exclusion semantics; prefix expansion is for positive lookups.
    if query.contains('"') || query.split_whitespace().any(|word| word.starts_with('-')) {
        return String::new();
    }
    let mut normalized = String::with_capacity(query.len());
    for token in query.split_whitespace() {
        if token.eq_ignore_ascii_case("or") {
            normalized.push_str("OR ");
            continue;
        }
        let words = stubs::search_words(token);
        for word in words.split_whitespace() {
            // “statuses” is the irregular plural used by status-listing tools.
            normalized.push_str(if word.eq_ignore_ascii_case("statuses") {
                "status"
            } else {
                word
            });
            normalized.push(' ');
        }
    }
    normalized
}

fn manifest(rows: Vec<ManifestTool>) -> String {
    struct Entry {
        name: String,
        signature: Option<String>,
        use_signature: bool,
    }
    let mut groups = std::collections::BTreeMap::<String, (i64, Vec<Entry>)>::new();
    for row in rows {
        let source_prefix = format!("async def tools.{}.", row.source);
        let group = groups
            .entry(row.source)
            .or_insert_with(|| (row.total, Vec::new()));
        if let Some(path) = row.path {
            let short = path
                .rsplit_once('.')
                .map_or(path.as_str(), |(_, name)| name);
            let mut name = short.to_owned();
            if row.blocked {
                name.push_str(" [blocked]");
            }
            let signature = row
                .brief
                .as_deref()
                .and_then(|brief| brief.lines().next())
                .and_then(|line| line.strip_prefix(&source_prefix))
                .and_then(|line| line.strip_suffix(": ..."))
                .filter(|line| !row.blocked && line.len() <= 480)
                .map(|line| {
                    let mut signature = line.to_owned();
                    if let Some(observed) = row.observed_shape {
                        signature.truncate(
                            signature
                                .rsplit_once(" -> ")
                                .map_or(signature.len(), |(head, _)| head.len()),
                        );
                        signature.push_str(" -> ");
                        signature.push_str(&observed);
                    } else if let Some(schema) = row.output_schema {
                        let suffix = format!(" -> {}", stubs::return_skeleton(&schema));
                        if !signature.ends_with(&suffix) {
                            signature.push_str(&suffix);
                        }
                    }
                    signature
                });
            group.1.push(Entry {
                name,
                signature,
                use_signature: false,
            });
        }
    }
    let count: i64 = groups.values().map(|(count, _)| *count).sum();
    let mut text = format!(
        "Connected sources ({count} tools), already loaded inside execute as tools.<source>.<tool>: "
    );
    if groups.is_empty() {
        text.push_str("none");
        return text;
    }

    // Reserve ranked names for every source first. Spend only the remainder on
    // signatures, one rank across all sources at a time.
    let total = groups.len();
    let mut remaining = 3072 - text.len() - 64;
    let mut visible = Vec::new();
    for (source, (count, entries)) in groups {
        let head = format!("{source} ({count}: ");
        let ellipsis = count > entries.len() as i64;
        let minimum = head.len()
            + 3
            + usize::from(ellipsis) * 5
            + entries
                .iter()
                .map(|entry| entry.name.len() + 2)
                .sum::<usize>();
        if let Some(left) = remaining.checked_sub(minimum) {
            remaining = left;
            visible.push((head, ellipsis, entries));
        }
    }
    for rank in 0..5 {
        for (_, _, entries) in &mut visible {
            if let Some(entry) = entries.get_mut(rank)
                && let Some(signature) = &entry.signature
                && let Some(left) =
                    remaining.checked_sub(signature.len().saturating_sub(entry.name.len()))
            {
                remaining = left;
                entry.use_signature = true;
            }
        }
    }
    let omitted = total - visible.len();
    for (index, (head, ellipsis, entries)) in visible.into_iter().enumerate() {
        if index != 0 {
            text.push_str("; ");
        }
        text.push_str(&head);
        let mut first = true;
        for entry in entries {
            if !first {
                text.push_str("; ");
            }
            first = false;
            text.push_str(
                entry
                    .signature
                    .as_deref()
                    .filter(|_| entry.use_signature)
                    .unwrap_or(&entry.name),
            );
        }
        if ellipsis {
            text.push_str(if first { "…" } else { "; …" });
        }
        text.push(')');
    }
    if omitted > 0 {
        text.push_str(&format!(
            " … {omitted} more sources; discover with tools.search"
        ));
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
                suggested: false,
                path: r.path,
                description: first_line(r.description),
                signature: r.signature,
                blocked: r.blocked,
            }
        })
        .collect();
    Ok(Page {
        tools,
        suggestions: Vec::new(),
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
                observed_shape: None,
                total: 15,
                blocked: false,
            },
            ManifestTool {
                source: "artifacts".into(),
                path: Some("artifacts.artifact_delete".into()),
                brief: None,
                output_schema: None,
                observed_shape: None,
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
                observed_shape: None,
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
    fn large_signatures_cannot_starve_the_last_source() {
        let rows = ["artifacts", "moneybag", "nudge"]
            .into_iter()
            .flat_map(|source| {
                (0..5).map(move |rank| {
                    let name = format!("{source}_rank_{rank}");
                    ManifestTool {
                        source: source.into(),
                        path: Some(format!("{source}.{name}")),
                        brief: Some(format!(
                            "async def tools.{source}.{name}(long: {}): ...",
                            "x".repeat(375)
                        )),
                        output_schema: None,
                        observed_shape: None,
                        total: 20,
                        blocked: false,
                    }
                })
            })
            .collect();
        let manifest = manifest(rows);
        assert!(manifest.len() <= 3072);
        for source in ["artifacts", "moneybag", "nudge"] {
            for rank in 0..5 {
                assert!(manifest.contains(&format!("{source}_rank_{rank}")));
            }
            assert!(manifest.contains(&format!("{source}_rank_0(")));
        }
        assert!(!manifest.contains("nudge_rank_4("));
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
