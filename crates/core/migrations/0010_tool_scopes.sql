ALTER TABLE endpoints ADD COLUMN all_tools boolean NOT NULL DEFAULT true;
CREATE TABLE endpoint_tools (
 endpoint_id uuid NOT NULL REFERENCES endpoints ON DELETE CASCADE,
 source_id uuid NOT NULL REFERENCES sources ON DELETE CASCADE,
 upstream_name text NOT NULL,
 PRIMARY KEY(endpoint_id,source_id,upstream_name)
);
CREATE INDEX endpoint_tools_source ON endpoint_tools(source_id);
