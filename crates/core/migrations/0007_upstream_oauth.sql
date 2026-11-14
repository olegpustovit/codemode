CREATE TABLE upstream_oauth_states (
 state_hash bytea PRIMARY KEY CHECK(octet_length(state_hash)=32),
 session_hash bytea NOT NULL CHECK(octet_length(session_hash)=32),
 source_id uuid NOT NULL REFERENCES sources ON DELETE CASCADE,
 source_revision bigint NOT NULL,
 ciphertext bytea NOT NULL,
 nonce bytea NOT NULL CHECK(octet_length(nonce)=24),
 key_id text NOT NULL,
 expires_at timestamptz NOT NULL DEFAULT now()+interval '10 minutes'
);
CREATE INDEX upstream_oauth_states_expiry ON upstream_oauth_states(expires_at);
CREATE INDEX upstream_oauth_states_source ON upstream_oauth_states(source_id);
