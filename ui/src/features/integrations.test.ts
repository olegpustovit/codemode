import { readFileSync } from "node:fs"
import { expect, test } from "vitest"
import { integrationSchema } from "./queries"

test("the UI accepts every method from the actual embedded registry", () => {
  const entries: unknown[] = JSON.parse(
    readFileSync("../crates/core/integrations.json", "utf8")
  )
  expect(entries.length).toBeGreaterThan(0)
  for (const entry of entries) {
    const parsed = integrationSchema.parse(
      Object.assign({}, entry, {
        sourceId: null,
        state: "available",
        toolCount: 0,
        error: null,
      })
    )
    expect(parsed.methods.length).toBeGreaterThan(0)
  }
  expect(
    integrationSchema.safeParse({ methods: ["unrecognized"] }).success
  ).toBe(false)
})

import { filterIntegrations } from "./integrations"
test("integration filters search vendor/category/tool names and retain link-state tabs", () => {
  const entry = {
    name: "GitHub",
    slug: "github",
    category: "Development",
    description: "Repositories",
    url: "https://api.githubcopilot.com/mcp/",
    docs: "",
    icon: "github",
    methods: ["api_key"],
    sourceId: "11111111-1111-4111-8111-111111111111",
    state: "ready",
    toolCount: 1,
    toolNames: ["github.create_issue"],
    error: null,
  }
  const entries = [
    integrationSchema.parse(entry),
    integrationSchema.parse({
      ...entry,
      name: "Notion",
      slug: "notion",
      state: "needs_link",
      category: "Knowledge",
      toolNames: [],
    }),
  ]
  expect(
    filterIntegrations(entries, "create issue", "all").map((e) => e.slug)
  ).toEqual(["github"])
  expect(
    filterIntegrations(entries, "Knowledge", "attention").map((e) => e.slug)
  ).toEqual(["notion"])
  expect(filterIntegrations(entries, "Knowledge", "linked")).toEqual([])
  expect(filterIntegrations(entries, "unknown vendor", "all")).toEqual([])
})
