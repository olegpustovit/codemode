-- Keep installation receipts after a user deletes a recipe, so it stays deleted.
CREATE TABLE source_recipe_installs (
 workspace_id uuid NOT NULL REFERENCES endpoints ON DELETE CASCADE,
 source_id uuid NOT NULL REFERENCES sources ON DELETE CASCADE,
 recipe text NOT NULL,
 PRIMARY KEY(workspace_id,source_id,recipe)
);
ALTER TABLE source_recipe_installs ENABLE ROW LEVEL SECURITY;
CREATE POLICY tenant ON source_recipe_installs USING(
 EXISTS(SELECT 1 FROM endpoints e WHERE e.id=workspace_id)
 AND EXISTS(SELECT 1 FROM sources s WHERE s.id=source_id)
);
GRANT SELECT,INSERT,UPDATE,DELETE ON source_recipe_installs TO codemode_tenant;
