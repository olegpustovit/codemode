ALTER TABLE sources DROP CONSTRAINT sources_kind_check;
ALTER TABLE sources ADD CONSTRAINT sources_kind_check CHECK(kind IN ('mcp_http','mcp_stdio','openapi','graphql'));
