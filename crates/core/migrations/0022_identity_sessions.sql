CREATE TABLE identity_sessions (
 token_hash bytea PRIMARY KEY CHECK(octet_length(token_hash)=32),
 subject text NOT NULL REFERENCES user_identities(subject) ON DELETE CASCADE,
 expires_at timestamptz NOT NULL DEFAULT now() + interval '12 hours'
);
CREATE INDEX identity_sessions_expiry ON identity_sessions(expires_at);
