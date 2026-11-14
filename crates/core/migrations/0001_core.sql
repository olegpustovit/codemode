CREATE TABLE sources (
 id uuid PRIMARY KEY DEFAULT gen_random_uuid(), slug text UNIQUE NOT NULL,
 kind text NOT NULL CHECK(kind IN ('mcp_http','mcp_stdio')),
 config jsonb NOT NULL, status text NOT NULL DEFAULT 'pending', last_error text,
 synced_at timestamptz, created_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE tools (
 id uuid PRIMARY KEY DEFAULT gen_random_uuid(), source_id uuid NOT NULL REFERENCES sources ON DELETE CASCADE,
 upstream_name text NOT NULL, path text UNIQUE NOT NULL, description text NOT NULL,
 input_schema jsonb NOT NULL, output_schema jsonb, stub_brief text NOT NULL, stub_full text NOT NULL,
 read_only boolean NOT NULL DEFAULT false, destructive boolean NOT NULL DEFAULT false,
 search tsvector NOT NULL, UNIQUE(source_id,upstream_name)
);
CREATE INDEX tools_search ON tools USING gin(search);
CREATE TABLE access_tokens (
 id uuid PRIMARY KEY DEFAULT gen_random_uuid(), name text NOT NULL, audience text NOT NULL,
 token_hash bytea UNIQUE NOT NULL CHECK(octet_length(token_hash)=32), created_at timestamptz NOT NULL DEFAULT now(),
 last_used_at timestamptz, revoked_at timestamptz
);
CREATE TABLE executions (
 id uuid PRIMARY KEY DEFAULT gen_random_uuid(), token_id uuid NOT NULL REFERENCES access_tokens,
 code text NOT NULL, status text NOT NULL CHECK(status IN ('running','completed','failed','timed_out','paused','expired')),
 error text, tool_calls integer NOT NULL DEFAULT 0, started_at timestamptz NOT NULL DEFAULT now(), finished_at timestamptz
);
CREATE TABLE tool_calls (
 id uuid PRIMARY KEY DEFAULT gen_random_uuid(), execution_id uuid NOT NULL REFERENCES executions ON DELETE CASCADE,
 seq integer NOT NULL, path text NOT NULL, args_redacted jsonb NOT NULL,
 outcome text NOT NULL, error text, started_at timestamptz NOT NULL DEFAULT now(), duration_ms bigint NOT NULL DEFAULT 0,
 UNIQUE(execution_id,seq)
);
