CREATE TABLE agent_session_memo (
 session_id uuid PRIMARY KEY REFERENCES agent_sessions ON DELETE CASCADE,
 tenant_id uuid NOT NULL REFERENCES tenants ON DELETE CASCADE,
 data jsonb NOT NULL DEFAULT '{}' CHECK(jsonb_typeof(data)='object' AND octet_length(data::text)<=65536)
);
ALTER TABLE agent_session_memo ENABLE ROW LEVEL SECURITY;
CREATE POLICY tenant ON agent_session_memo
 USING(tenant_id=NULLIF(current_setting('codemode.tenant_id',true),'')::uuid)
 WITH CHECK(tenant_id=NULLIF(current_setting('codemode.tenant_id',true),'')::uuid AND EXISTS(
  SELECT 1 FROM agent_sessions s JOIN access_tokens a ON a.id=s.token_id
  WHERE s.id=session_id AND a.tenant_id=agent_session_memo.tenant_id
 ));
GRANT SELECT,INSERT,UPDATE,DELETE ON agent_session_memo TO codemode_tenant;
