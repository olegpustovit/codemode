CREATE TABLE tool_policies (
 endpoint_id uuid NOT NULL REFERENCES endpoints ON DELETE CASCADE,
 source_id uuid NOT NULL REFERENCES sources ON DELETE CASCADE,
 upstream_name text NOT NULL,
 action text NOT NULL CHECK(action IN ('allow','approve','block')),
 PRIMARY KEY(endpoint_id,source_id,upstream_name)
);
CREATE INDEX tool_policies_source ON tool_policies(source_id);
CREATE TABLE paused_runs (
 execution_id uuid PRIMARY KEY REFERENCES executions ON DELETE CASCADE,
 pause_id uuid UNIQUE NOT NULL,
 token_id uuid NOT NULL REFERENCES access_tokens,
 audience text NOT NULL,
 expires_at timestamptz NOT NULL,
 context bytea NOT NULL CHECK(octet_length(context)<=9437184),
 snapshot bytea NOT NULL CHECK(octet_length(snapshot)<=75497472),
 signature bytea NOT NULL CHECK(octet_length(signature)=32)
);
CREATE INDEX paused_runs_expiry ON paused_runs(expires_at);
CREATE TABLE approval_calls (
 execution_id uuid NOT NULL REFERENCES executions ON DELETE CASCADE,
 pause_id uuid NOT NULL,
 call_id bigint NOT NULL CHECK(call_id BETWEEN 0 AND 4294967295),
 path text NOT NULL,
 args_redacted jsonb NOT NULL,
 error text,
 decision boolean,
 decided_at timestamptz,
 PRIMARY KEY(pause_id,call_id)
);
CREATE INDEX approval_calls_execution ON approval_calls(execution_id);
