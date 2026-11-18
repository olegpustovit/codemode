CREATE EXTENSION IF NOT EXISTS pg_trgm;

ALTER TABLE tools
    ADD COLUMN observed_shape text,
    ADD COLUMN observed_revision bigint,
    ADD CONSTRAINT tools_observed_shape_budget CHECK (octet_length(observed_shape) <= 512);
