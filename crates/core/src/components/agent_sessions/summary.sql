SELECT s.id,s.token_id,s.workspace_id,s.client,
 COALESCE(s.title,a.first_title,'Python execution') AS "title!",
 COALESCE(w.slug,'Deleted workspace') AS "workspace_name!",
 to_char(s.started_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS "started_at!",
 to_char(s.last_seen_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS "last_seen_at!",
 to_char(s.expires_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS "expires_at!",
 a.executions AS "executions!",a.tool_calls AS "tool_calls!",a.failures AS "failures!",
 CASE WHEN a.running>0 THEN 'running' WHEN a.failures>0 THEN 'failed' ELSE 'completed' END AS "status!",
 COALESCE(c.sources,ARRAY[]::text[]) AS "sources!"
FROM agent_sessions s LEFT JOIN endpoints w ON w.id=s.workspace_id
CROSS JOIN LATERAL (
 SELECT count(*) AS executions,COALESCE(sum(e.tool_calls),0)::bigint AS tool_calls,
 count(*) FILTER(WHERE e.status IN ('failed','timed_out','expired')) AS failures,
 count(*) FILTER(WHERE e.status='running') AS running,
 (array_agg(e.title ORDER BY e.started_at,e.id))[1] AS first_title
 FROM executions e WHERE e.session_id=s.id
) a
CROSS JOIN LATERAL (
 SELECT array_agg(DISTINCT split_part(t.path,'.',1) ORDER BY split_part(t.path,'.',1)) AS sources
 FROM tool_calls t JOIN executions e ON e.id=t.execution_id WHERE e.session_id=s.id
) c
WHERE s.expires_at>now() AND ($1::uuid IS NULL OR s.id=$1)
 AND ($2='' OR concat_ws(' ',s.title,a.first_title,s.client,w.slug,array_to_string(c.sources,' '),
 CASE WHEN a.running>0 THEN 'running' WHEN a.failures>0 THEN 'failed' ELSE 'completed' END) ILIKE '%'||$2||'%')
 AND ($3::uuid IS NULL OR (s.last_seen_at,s.id)<(SELECT last_seen_at,id FROM agent_sessions WHERE id=$3))
ORDER BY s.last_seen_at DESC,s.id DESC LIMIT 51
