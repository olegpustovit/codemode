import { readFileSync } from "node:fs"
import { expect, test } from "vitest"
import { matches, matchRanges } from "./match"
import { z } from "zod"
const cases = z
  .array(
    z.object({ query: z.string(), value: z.string(), matches: z.boolean() })
  )
  .parse(JSON.parse(readFileSync("../fixtures/search-patterns.json", "utf8")))
test("glob and literal search follows the shared browser/server cases", () => {
  for (const item of cases)
    expect(matches(item.query, item.value), JSON.stringify(item)).toBe(
      item.matches
    )
  expect(matches("github.* pull", "GitHub", "github.list_pull_requests")).toBe(
    true
  )
})
test("highlight literal pieces, merge overlapping terms and leave wildcards unmarked", () => {
  expect(matchRanges("github.*", "github.list_issues")).toEqual([
    { start: 0, end: 7 },
  ])
  expect(matchRanges("*.delete_*", "github.delete_issue")).toEqual([
    { start: 6, end: 14 },
  ])
  expect(matchRanges("issue iss", "list_issues")).toEqual([
    { start: 5, end: 10 },
  ])
  expect(matchRanges("?", "é")).toEqual([])
  expect(matchRanges("İ*", "İd")).toEqual([{ start: 0, end: 1 }])
  expect(matchRanges("i", "İd")).toEqual([{ start: 0, end: 1 }])
  expect(matchRanges("<script>", "<script>")).toEqual([{ start: 0, end: 8 }])
})
test("long unsuccessful wildcard patterns do not backtrack exponentially", () => {
  expect(matches("a*".repeat(200) + "b", "a".repeat(2000))).toBe(false)
})
