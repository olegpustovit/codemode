ALTER TABLE saved_scripts ADD COLUMN tenant_id uuid REFERENCES tenants;
UPDATE saved_scripts AS s SET tenant_id=e.tenant_id FROM endpoints AS e WHERE e.id=s.endpoint_id;
ALTER TABLE saved_scripts ALTER COLUMN tenant_id SET NOT NULL,
 ALTER COLUMN endpoint_id DROP NOT NULL,
 ADD COLUMN saved_by text CHECK(char_length(saved_by)<=120),
 ADD COLUMN from_execution uuid REFERENCES executions ON DELETE SET NULL;
CREATE UNIQUE INDEX saved_scripts_global_name ON saved_scripts(tenant_id,name) WHERE endpoint_id IS NULL;

-- Older gateways still insert workspace scripts during a binary rollback.
CREATE FUNCTION saved_script_tenant() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE endpoint_tenant uuid;
BEGIN
 IF NEW.endpoint_id IS NOT NULL THEN
  SELECT tenant_id INTO STRICT endpoint_tenant FROM endpoints WHERE id=NEW.endpoint_id;
  IF NEW.tenant_id IS NOT NULL AND NEW.tenant_id<>endpoint_tenant THEN
   RAISE EXCEPTION 'script and workspace must belong to the same tenant' USING ERRCODE='23514';
  END IF;
  NEW.tenant_id := endpoint_tenant;
 END IF;
 RETURN NEW;
END;
$$;
CREATE TRIGGER saved_script_tenant BEFORE INSERT OR UPDATE OF endpoint_id,tenant_id ON saved_scripts
 FOR EACH ROW EXECUTE FUNCTION saved_script_tenant();
DROP POLICY tenant ON saved_scripts;
CREATE POLICY tenant ON saved_scripts USING(tenant_id=NULLIF(current_setting('codemode.tenant_id',true),'')::uuid);

ALTER TABLE control_changes DROP CONSTRAINT control_changes_action_check;
ALTER TABLE control_changes ADD CONSTRAINT control_changes_action_check
 CHECK(action IN ('allow','block','include','exclude','revoke','delete','sync','promote','demote'));

ALTER TABLE oauth_grants ADD COLUMN admin boolean NOT NULL DEFAULT false;
ALTER TABLE access_tokens ADD COLUMN admin boolean NOT NULL DEFAULT false;
UPDATE oauth_grants SET revoked_at=COALESCE(revoked_at,now()) WHERE scope='admin';
UPDATE access_tokens SET revoked_at=COALESCE(revoked_at,now()) WHERE resource='/mcp-admin';
