ALTER TABLE sources ADD COLUMN name text;
-- The previous binary can still insert sources after a binary rollback.
CREATE FUNCTION source_display_default() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 NEW.name := COALESCE(NEW.name,NEW.slug);
 RETURN NEW;
END;
$$;
CREATE TRIGGER source_display_default BEFORE INSERT ON sources
 FOR EACH ROW EXECUTE FUNCTION source_display_default();
UPDATE sources SET name=slug;
ALTER TABLE sources ALTER COLUMN name SET NOT NULL;
ALTER TABLE sources ADD CONSTRAINT source_name_length CHECK (char_length(name) BETWEEN 1 AND 120 AND length(btrim(name)) > 0);
ALTER TABLE sources ADD COLUMN icon text NOT NULL DEFAULT 'plug'
 CHECK (icon IN ('plug','database','cloud','bolt','chart','mail','calendar','card','box','bug','book','globe','terminal','key','users','file','mono'));

CREATE TABLE control_changes (
 id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
 tenant_id uuid NOT NULL DEFAULT COALESCE(NULLIF(current_setting('codemode.tenant_id',true),''),'00000000-0000-0000-0000-000000000000')::uuid REFERENCES tenants,
 request_id uuid NOT NULL,
 workspace_id uuid,
 kind text NOT NULL CHECK(kind IN ('policy','scope','token','script','source')),
 item text NOT NULL,
 action text NOT NULL CHECK(action IN ('allow','block','include','exclude','revoke','delete','sync')),
 changed_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX control_changes_tenant_time ON control_changes(tenant_id,changed_at);
ALTER TABLE control_changes ENABLE ROW LEVEL SECURITY;
CREATE POLICY tenant ON control_changes USING(tenant_id=NULLIF(current_setting('codemode.tenant_id',true),'')::uuid);
GRANT SELECT,INSERT ON control_changes TO codemode_tenant;
