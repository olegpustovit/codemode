-- Existing data is adopted by the first account; no account is provisioned by deployment.
CREATE TABLE tenants (id uuid PRIMARY KEY DEFAULT gen_random_uuid(), created_at timestamptz NOT NULL DEFAULT now());
INSERT INTO tenants(id) VALUES('00000000-0000-0000-0000-000000000000');
CREATE TABLE users (
 id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
 email text UNIQUE NOT NULL CHECK(email=lower(email)),
 password_hash text NOT NULL,
 tenant_id uuid UNIQUE NOT NULL REFERENCES tenants,
 admin boolean NOT NULL DEFAULT false,
 created_at timestamptz NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX users_first_admin ON users(admin) WHERE admin;
DELETE FROM sessions;
ALTER TABLE sessions ADD COLUMN user_id uuid NOT NULL REFERENCES users ON DELETE CASCADE;
-- Browser authentication and agent credentials issued before private tenancy cannot survive adoption.
UPDATE access_tokens SET revoked_at=COALESCE(revoked_at,now());
UPDATE oauth_grants SET revoked_at=COALESCE(revoked_at,now());

ALTER TABLE sources ADD COLUMN tenant_id uuid NOT NULL DEFAULT COALESCE(NULLIF(current_setting('codemode.tenant_id',true),''),'00000000-0000-0000-0000-000000000000')::uuid REFERENCES tenants;
ALTER TABLE endpoints ADD COLUMN tenant_id uuid NOT NULL DEFAULT COALESCE(NULLIF(current_setting('codemode.tenant_id',true),''),'00000000-0000-0000-0000-000000000000')::uuid REFERENCES tenants;
ALTER TABLE access_tokens ADD COLUMN tenant_id uuid NOT NULL DEFAULT COALESCE(NULLIF(current_setting('codemode.tenant_id',true),''),'00000000-0000-0000-0000-000000000000')::uuid REFERENCES tenants;
ALTER TABLE sources DROP CONSTRAINT sources_slug_key;
ALTER TABLE sources ADD UNIQUE(tenant_id,slug);
DROP INDEX sources_python_name;
CREATE UNIQUE INDEX sources_python_name ON sources(tenant_id,python_name) WHERE python_name IS NOT NULL;
ALTER TABLE tools DROP CONSTRAINT tools_path_key;
ALTER TABLE tools ADD UNIQUE(source_id,path);
ALTER TABLE endpoints DROP CONSTRAINT endpoints_slug_key;
ALTER TABLE endpoints DROP CONSTRAINT endpoints_hostname_key;
ALTER TABLE endpoints ADD UNIQUE(tenant_id,hostname);
CREATE UNIQUE INDEX endpoints_named_host ON endpoints(hostname) WHERE NOT all_sources;
DROP INDEX endpoints_main;
CREATE UNIQUE INDEX endpoints_main ON endpoints(tenant_id) WHERE all_sources;

-- Each tenant pool assumes an unprivileged role once, before any application query.
DO $$ BEGIN
 CREATE ROLE codemode_tenant NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOBYPASSRLS;
EXCEPTION WHEN duplicate_object OR unique_violation THEN NULL; END $$;
DO $$ BEGIN
 IF EXISTS(SELECT 1 FROM pg_roles WHERE rolname='codemode_tenant' AND (rolsuper OR rolbypassrls OR rolcanlogin OR rolcreaterole OR rolcreatedb)) THEN
  RAISE EXCEPTION 'codemode_tenant must be an unprivileged NOLOGIN role';
 END IF;
 EXECUTE format('GRANT codemode_tenant TO %I',current_user);
END $$;
GRANT USAGE ON SCHEMA public TO codemode_tenant;
GRANT SELECT,INSERT,UPDATE,DELETE ON sources,tools,endpoints,endpoint_sources,endpoint_tools,tool_policies,accounts,upstream_oauth_states,access_tokens,executions,tool_calls,paused_runs,approval_calls,saved_scripts,oauth_clients,oauth_grants,oauth_codes,oauth_tokens TO codemode_tenant;

ALTER TABLE sources ENABLE ROW LEVEL SECURITY;
CREATE POLICY tenant ON sources USING(tenant_id=NULLIF(current_setting('codemode.tenant_id',true),'')::uuid);
ALTER TABLE endpoints ENABLE ROW LEVEL SECURITY;
CREATE POLICY tenant ON endpoints USING(tenant_id=NULLIF(current_setting('codemode.tenant_id',true),'')::uuid);
ALTER TABLE access_tokens ENABLE ROW LEVEL SECURITY;
CREATE POLICY tenant ON access_tokens USING(tenant_id=NULLIF(current_setting('codemode.tenant_id',true),'')::uuid);
ALTER TABLE tools ENABLE ROW LEVEL SECURITY;
CREATE POLICY tenant ON tools USING(EXISTS(SELECT 1 FROM sources s WHERE s.id=source_id));
ALTER TABLE accounts ENABLE ROW LEVEL SECURITY;
CREATE POLICY tenant ON accounts USING(EXISTS(SELECT 1 FROM sources s WHERE s.id=source_id));
ALTER TABLE upstream_oauth_states ENABLE ROW LEVEL SECURITY;
CREATE POLICY tenant ON upstream_oauth_states USING(EXISTS(SELECT 1 FROM sources s WHERE s.id=source_id));
ALTER TABLE endpoint_sources ENABLE ROW LEVEL SECURITY;
CREATE POLICY tenant ON endpoint_sources USING(EXISTS(SELECT 1 FROM endpoints e WHERE e.id=endpoint_id) AND EXISTS(SELECT 1 FROM sources s WHERE s.id=source_id));
ALTER TABLE endpoint_tools ENABLE ROW LEVEL SECURITY;
CREATE POLICY tenant ON endpoint_tools USING(EXISTS(SELECT 1 FROM endpoints e WHERE e.id=endpoint_id) AND EXISTS(SELECT 1 FROM sources s WHERE s.id=source_id));
ALTER TABLE tool_policies ENABLE ROW LEVEL SECURITY;
CREATE POLICY tenant ON tool_policies USING(EXISTS(SELECT 1 FROM endpoints e WHERE e.id=endpoint_id) AND EXISTS(SELECT 1 FROM sources s WHERE s.id=source_id));
ALTER TABLE executions ENABLE ROW LEVEL SECURITY;
CREATE POLICY tenant ON executions USING(EXISTS(SELECT 1 FROM access_tokens a WHERE a.id=token_id));
ALTER TABLE tool_calls ENABLE ROW LEVEL SECURITY;
CREATE POLICY tenant ON tool_calls USING(EXISTS(SELECT 1 FROM executions e WHERE e.id=execution_id));
ALTER TABLE paused_runs ENABLE ROW LEVEL SECURITY;
CREATE POLICY tenant ON paused_runs USING(EXISTS(SELECT 1 FROM executions e WHERE e.id=execution_id) AND EXISTS(SELECT 1 FROM access_tokens a WHERE a.id=token_id));
ALTER TABLE approval_calls ENABLE ROW LEVEL SECURITY;
CREATE POLICY tenant ON approval_calls USING(EXISTS(SELECT 1 FROM executions e WHERE e.id=execution_id));
ALTER TABLE saved_scripts ENABLE ROW LEVEL SECURITY;
CREATE POLICY tenant ON saved_scripts USING(EXISTS(SELECT 1 FROM endpoints e WHERE e.id=endpoint_id));
ALTER TABLE oauth_grants ENABLE ROW LEVEL SECURITY;
CREATE POLICY tenant ON oauth_grants USING(EXISTS(SELECT 1 FROM endpoints e WHERE e.id=endpoint_id));
ALTER TABLE oauth_codes ENABLE ROW LEVEL SECURITY;
CREATE POLICY tenant ON oauth_codes USING(EXISTS(SELECT 1 FROM oauth_grants g WHERE g.id=grant_id));
ALTER TABLE oauth_tokens ENABLE ROW LEVEL SECURITY;
CREATE POLICY tenant ON oauth_tokens USING(EXISTS(SELECT 1 FROM oauth_grants g WHERE g.id=grant_id));
