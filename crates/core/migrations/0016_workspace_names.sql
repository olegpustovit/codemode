-- Names are tenant-local identities now that multiple workspaces share one host.
CREATE UNIQUE INDEX workspaces_tenant_name ON endpoints(tenant_id,slug);
