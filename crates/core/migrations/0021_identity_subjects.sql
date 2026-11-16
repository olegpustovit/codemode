-- Private global authentication mapping; tenant-scoped connections get no access.
CREATE TABLE user_identities (
 subject text PRIMARY KEY CHECK(octet_length(subject) BETWEEN 1 AND 255),
 user_id uuid UNIQUE NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 created_at timestamptz NOT NULL DEFAULT now()
);
