-- Keep existing word/description weights and add exact tool names for camelCase and snake_case searches.
UPDATE tools SET search = setweight(to_tsvector('simple',path || ' ' || upstream_name || ' ' || split_part(path,'.',2)),'A') || search;
