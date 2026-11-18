import { expect, test } from "vitest"
import { mcpUrl, workspaceId } from "./workspace-url"

test("accepts only canonical lowercase workspace IDs in MCP URLs", () => {
  const id = "abcdef01-2345-4678-9abc-def012345678"
  expect(workspaceId(id)).toBe(id)
  expect(mcpUrl("codemode.microapps.space", id)).toBe(
    `https://codemode.microapps.space/mcp?workspace=${id}`
  )
  for (const invalid of [null, [id], id.toUpperCase(), `${id}&other=true`]) {
    expect(workspaceId(invalid)).toBeUndefined()
  }
})
