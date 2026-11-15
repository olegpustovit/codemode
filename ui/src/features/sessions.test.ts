import { expect, test } from "vitest"
import { timelinePosition } from "./sessions"
test("timeline uses the actual four-day window and clips activity outside it", () => {
  const now = Date.parse("2026-11-15T12:00:00Z")
  expect(timelinePosition("2026-11-11T12:00:00Z", now)).toBe(0)
  expect(timelinePosition("2026-11-13T12:00:00Z", now)).toBe(50)
  expect(timelinePosition(now, now)).toBe(100)
  expect(timelinePosition("2026-11-10T12:00:00Z", now)).toBe(0)
  expect(timelinePosition("2026-11-16T12:00:00Z", now)).toBe(100)
})
