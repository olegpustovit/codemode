import { expect, test } from "vitest"
import { relativeTime } from "./time"

test("lists switch from relative time to compact dates after a week", () => {
  const now = new Date(2026, 10, 20, 14, 3).getTime()
  expect(relativeTime(new Date(2026, 10, 19, 14, 3).toISOString(), now)).toBe(
    "yesterday"
  )
  expect(relativeTime(new Date(2026, 10, 12, 14, 3).toISOString(), now)).toBe(
    "Nov 12 14:03"
  )
  expect(relativeTime(new Date(2025, 10, 12, 14, 3).toISOString(), now)).toBe(
    "Nov 12 2025 14:03"
  )
})
