CREATE TABLE oauth_clients (
 client_id text PRIMARY KEY,
 metadata jsonb NOT NULL,
 secret_hash bytea CHECK(secret_hash IS NULL OR octet_length(secret_hash)=32),
 updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE oauth_grants (
 id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
 client_id text NOT NULL REFERENCES oauth_clients,
 endpoint_id uuid NOT NULL REFERENCES endpoints ON DELETE CASCADE,
 scope text NOT NULL,
 refresh_allowed boolean NOT NULL,
 created_at timestamptz NOT NULL DEFAULT now(),
 expires_at timestamptz NOT NULL DEFAULT now()+interval '30 days',
 revoked_at timestamptz
);
CREATE INDEX oauth_grants_endpoint ON oauth_grants(endpoint_id);
CREATE TABLE oauth_codes (
 code_hash bytea PRIMARY KEY CHECK(octet_length(code_hash)=32),
 grant_id uuid NOT NULL REFERENCES oauth_grants ON DELETE CASCADE,
 redirect_uri text NOT NULL,
 challenge text NOT NULL CHECK(length(challenge)=43),
 expires_at timestamptz NOT NULL DEFAULT now()+interval '5 minutes',
 consumed_at timestamptz
);
CREATE INDEX oauth_codes_grant ON oauth_codes(grant_id);
CREATE TABLE oauth_tokens (
 token_hash bytea PRIMARY KEY CHECK(octet_length(token_hash)=32),
 grant_id uuid NOT NULL REFERENCES oauth_grants ON DELETE CASCADE,
 consumed_at timestamptz
);
CREATE INDEX oauth_tokens_grant ON oauth_tokens(grant_id);
ALTER TABLE access_tokens ADD COLUMN expires_at timestamptz;
ALTER TABLE access_tokens ADD COLUMN oauth_grant uuid REFERENCES oauth_grants ON DELETE SET NULL;
CREATE INDEX access_tokens_oauth_grant ON access_tokens(oauth_grant) WHERE oauth_grant IS NOT NULL;
