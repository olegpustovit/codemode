CREATE TABLE endpoints (
 id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
 slug text UNIQUE NOT NULL,
 hostname text UNIQUE NOT NULL,
 all_sources boolean NOT NULL DEFAULT false,
 created_at timestamptz NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX endpoints_main ON endpoints(all_sources) WHERE all_sources;
CREATE TABLE endpoint_sources (
 endpoint_id uuid NOT NULL REFERENCES endpoints ON DELETE CASCADE,
 source_id uuid NOT NULL REFERENCES sources ON DELETE CASCADE,
 PRIMARY KEY(endpoint_id,source_id)
);
CREATE INDEX endpoint_sources_source ON endpoint_sources(source_id);
