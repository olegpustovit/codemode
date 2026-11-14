import { describe, it, expect } from "vitest"
import { safeReturnPath, httpUrl } from "./api"
describe("navigation boundaries", () => {
  it("keeps login return paths inside control-plane routes", () => {
    for (const path of [
      "/sources",
      "/authorize?client_id=https%3A%2F%2Fclient.example",
      "/executions/123",
      "/approvals/123",
    ])
      expect(safeReturnPath(path)).toBe(path)
    for (const path of [
      "https://evil.example",
      "//evil.example",
      "/\\evil.example",
      "/api/tokens",
      "/sources\n",
      undefined,
    ])
      expect(safeReturnPath(path)).toBe("/sources")
  })
  it("does not navigate to active-content OAuth redirect URLs", () => {
    expect(
      httpUrl.safeParse("https://client.example/callback?code=x").success
    ).toBe(true)
    expect(httpUrl.safeParse("javascript:alert(1)").success).toBe(false)
  })
})
