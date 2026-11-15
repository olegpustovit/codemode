-- Tenant OAuth applications are independent of linked account credentials.
CREATE TABLE oauth_apps (
 id uuid PRIMARY KEY,
 tenant_id uuid NOT NULL DEFAULT COALESCE(NULLIF(current_setting('codemode.tenant_id',true),''),'00000000-0000-0000-0000-000000000000')::uuid REFERENCES tenants,
 provider text NOT NULL CHECK(provider ~ '^[a-z0-9_-]{1,64}$'),
 ciphertext bytea NOT NULL CHECK(octet_length(ciphertext)>=16),
 nonce bytea NOT NULL CHECK(octet_length(nonce)=24),
 key_id text NOT NULL,
 updated_at timestamptz NOT NULL DEFAULT now(),
 UNIQUE(tenant_id,provider)
);
ALTER TABLE oauth_apps ENABLE ROW LEVEL SECURITY;
CREATE POLICY tenant ON oauth_apps USING(tenant_id=COALESCE(NULLIF(current_setting('codemode.tenant_id',true),''),'00000000-0000-0000-0000-000000000000')::uuid);
GRANT SELECT,INSERT,UPDATE,DELETE ON oauth_apps TO codemode_tenant;
