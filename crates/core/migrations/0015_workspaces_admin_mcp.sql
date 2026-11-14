-- Keep existing IDs, membership, credentials, policies, scripts and audit links.
-- Legacy table/column names remain internal; workspaces no longer represent hostnames.
ALTER TABLE access_tokens ADD COLUMN workspace_id uuid REFERENCES endpoints ON DELETE SET NULL;
ALTER TABLE access_tokens ADD COLUMN resource text NOT NULL DEFAULT '/mcp' CHECK(resource IN ('/mcp','/mcp-admin'));
UPDATE access_tokens a SET workspace_id=e.id FROM endpoints e WHERE e.tenant_id=a.tenant_id AND e.hostname=a.audience;
-- A legacy custom-host grant must not silently change its OAuth resource.
UPDATE oauth_grants g SET revoked_at=COALESCE(g.revoked_at,now()) FROM endpoints e WHERE g.endpoint_id=e.id AND NOT e.all_sources;
UPDATE access_tokens a SET revoked_at=COALESCE(a.revoked_at,now()) WHERE a.workspace_id IS NULL OR EXISTS(SELECT 1 FROM endpoints e WHERE e.id=a.workspace_id AND NOT e.all_sources);
ALTER TABLE endpoints DROP CONSTRAINT endpoints_tenant_id_hostname_key;
DROP INDEX endpoints_named_host;
UPDATE endpoints e SET hostname=m.hostname FROM endpoints m WHERE m.tenant_id=e.tenant_id AND m.all_sources;
UPDATE endpoints SET slug='All sources' WHERE all_sources AND NOT EXISTS(SELECT 1 FROM endpoints other WHERE other.tenant_id=endpoints.tenant_id AND other.slug='All sources' AND other.id<>endpoints.id);
CREATE INDEX access_tokens_workspace ON access_tokens(workspace_id);

CREATE TABLE administration_calls (
 id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
 token_id uuid NOT NULL REFERENCES access_tokens,
 tool text NOT NULL,
 args_redacted jsonb NOT NULL,
 outcome text NOT NULL CHECK(outcome IN ('running','completed','failed')),
 started_at timestamptz NOT NULL DEFAULT now(),
 finished_at timestamptz
);
ALTER TABLE administration_calls ENABLE ROW LEVEL SECURITY;
CREATE POLICY tenant ON administration_calls USING(EXISTS(SELECT 1 FROM access_tokens a WHERE a.id=token_id));
GRANT SELECT,INSERT,UPDATE ON administration_calls TO codemode_tenant;
