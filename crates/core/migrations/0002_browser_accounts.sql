CREATE TABLE sessions (
 token_hash bytea PRIMARY KEY CHECK(octet_length(token_hash)=32),
 password_version bytea NOT NULL CHECK(octet_length(password_version)=32),
 expires_at timestamptz NOT NULL DEFAULT now() + interval '12 hours'
);
CREATE INDEX sessions_expiry ON sessions(expires_at);

CREATE TABLE accounts (
 source_id uuid PRIMARY KEY REFERENCES sources ON DELETE CASCADE,
 kind text NOT NULL CHECK(kind IN ('bearer','oauth')),
 ciphertext bytea NOT NULL CHECK(octet_length(ciphertext)>=16),
 nonce bytea NOT NULL CHECK(octet_length(nonce)=24),
 key_id text NOT NULL,
 expires_at timestamptz,
 updated_at timestamptz NOT NULL DEFAULT now()
);
