CREATE TABLE agent_sessions (
 id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
 token_id uuid NOT NULL REFERENCES access_tokens ON DELETE CASCADE,
 workspace_id uuid NOT NULL,
 oauth_grant uuid,
 client text NOT NULL,
 title text CHECK(title IS NULL OR char_length(btrim(title)) BETWEEN 1 AND 160),
 started_at timestamptz NOT NULL DEFAULT now(),
 last_seen_at timestamptz NOT NULL DEFAULT now(),
 -- Use UTC arithmetic so the stored expression is immutable and four days always means 96 hours.
 expires_at timestamptz GENERATED ALWAYS AS ((last_seen_at AT TIME ZONE 'UTC' + interval '4 days') AT TIME ZONE 'UTC') STORED
);
CREATE INDEX agent_sessions_token ON agent_sessions(token_id,workspace_id,last_seen_at DESC,id DESC);
CREATE INDEX agent_sessions_grant ON agent_sessions(oauth_grant,workspace_id,last_seen_at DESC,id DESC) WHERE oauth_grant IS NOT NULL;
CREATE INDEX agent_sessions_expiry ON agent_sessions(expires_at);
ALTER TABLE agent_sessions ENABLE ROW LEVEL SECURITY;
CREATE POLICY tenant ON agent_sessions USING(EXISTS(SELECT 1 FROM access_tokens a WHERE a.id=token_id));
GRANT SELECT,INSERT,UPDATE,DELETE ON agent_sessions TO codemode_tenant;

ALTER TABLE executions ADD COLUMN session_id uuid REFERENCES agent_sessions ON DELETE CASCADE,
 ADD COLUMN title text,
 ADD COLUMN result text CHECK(result IS NULL OR octet_length(result)<=16384),
 ADD COLUMN output text CHECK(output IS NULL OR octet_length(output)<=16384);
CREATE INDEX executions_session ON executions(session_id,started_at DESC,id DESC);
ALTER TABLE tool_calls ADD COLUMN preview text CHECK(preview IS NULL OR octet_length(preview)<=2048);

UPDATE executions e SET title=left(COALESCE(
 (SELECT btrim(regexp_replace(line,'^[[:blank:]]*#[[:blank:]]*',''),E' \t\r')
  FROM regexp_split_to_table(e.code,E'\n') WITH ORDINALITY AS lines(line,position)
  WHERE line ~ '^[[:blank:]]*#[[:blank:]]*[^[:space:]]' ORDER BY position LIMIT 1),
 (SELECT string_agg(path,' → ' ORDER BY seq) FROM tool_calls WHERE execution_id=e.id),
 'Python execution'),160);

CREATE TEMP TABLE codemode_session_backfill ON COMMIT DROP AS
WITH activity AS (
 SELECT e.id,e.token_id,COALESCE(a.oauth_grant,a.id) AS identity,a.oauth_grant,
 COALESCE(a.workspace_id,w.id) AS workspace_id,a.name AS client,e.title,e.started_at,
 GREATEST(e.started_at,e.finished_at) AS activity_at
 FROM executions e JOIN access_tokens a ON a.id=e.token_id
 LEFT JOIN endpoints w ON w.tenant_id=a.tenant_id AND w.all_sources
), previous AS (
 SELECT *,max(activity_at) OVER(PARTITION BY identity,workspace_id ORDER BY started_at,id ROWS BETWEEN UNBOUNDED PRECEDING AND 1 PRECEDING) AS previous_seen
 FROM activity
), numbered AS (
 SELECT *,sum(CASE WHEN previous_seen IS NULL OR started_at>previous_seen+interval '30 minutes' THEN 1 ELSE 0 END)
 OVER(PARTITION BY identity,workspace_id ORDER BY started_at,id) AS boundary
 FROM previous
)
SELECT *,first_value(id) OVER(PARTITION BY identity,workspace_id,boundary ORDER BY started_at,id) AS session_id FROM numbered;

INSERT INTO agent_sessions(id,token_id,workspace_id,oauth_grant,client,title,started_at,last_seen_at)
SELECT session_id,(array_agg(token_id ORDER BY started_at,id))[1],workspace_id,
 (array_agg(oauth_grant ORDER BY started_at,id))[1],(array_agg(client ORDER BY started_at,id))[1],
 (array_agg(title ORDER BY started_at,id))[1],min(started_at),max(activity_at)
FROM codemode_session_backfill GROUP BY session_id,workspace_id;
UPDATE executions e SET session_id=b.session_id FROM codemode_session_backfill b WHERE e.id=b.id;
DELETE FROM agent_sessions WHERE expires_at<=now();
