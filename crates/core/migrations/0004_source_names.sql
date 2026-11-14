ALTER TABLE sources ADD COLUMN python_name text;
CREATE UNIQUE INDEX sources_python_name ON sources(python_name) WHERE python_name IS NOT NULL;
