CREATE TABLE saved_scripts (
 id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
 endpoint_id uuid NOT NULL REFERENCES endpoints ON DELETE CASCADE,
 name text NOT NULL CHECK(char_length(name) BETWEEN 1 AND 120),
 description text NOT NULL CHECK(char_length(description)<=2000),
 code text NOT NULL CHECK(octet_length(code)<=65536),
 revision bigint NOT NULL DEFAULT 1,
 updated_at timestamptz NOT NULL DEFAULT now(),
 search tsvector GENERATED ALWAYS AS (setweight(to_tsvector('simple',name),'A') || setweight(to_tsvector('simple',description),'B')) STORED,
 UNIQUE(endpoint_id,name)
);
CREATE INDEX saved_scripts_endpoint ON saved_scripts(endpoint_id);
CREATE INDEX saved_scripts_search ON saved_scripts USING gin(search);
