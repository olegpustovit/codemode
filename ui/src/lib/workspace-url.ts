const canonicalUuid = /^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/

export function workspaceId(value: unknown) {
  return typeof value === "string" && canonicalUuid.test(value)
    ? value
    : undefined
}

export function mcpUrl(hostname: string, workspace?: string) {
  const id = workspaceId(workspace)
  return `https://${hostname}/mcp${id ? `?workspace=${id}` : ""}`
}
