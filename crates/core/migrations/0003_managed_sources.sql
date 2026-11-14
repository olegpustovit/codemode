ALTER TABLE sources ADD COLUMN revision bigint NOT NULL DEFAULT 0;
ALTER TABLE accounts DROP CONSTRAINT accounts_kind_check;
ALTER TABLE accounts ADD CONSTRAINT accounts_kind_check CHECK(kind IN ('bearer','oauth','environment'));
CREATE TABLE source_import (
 singleton boolean PRIMARY KEY CHECK(singleton),
 completed_at timestamptz NOT NULL DEFAULT now()
);
