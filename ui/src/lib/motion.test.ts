import { afterEach, expect, test, vi } from "vitest"
import { move } from "./motion"

afterEach(() => vi.unstubAllGlobals())

test("uses the requested transition type and immediately honors reduced motion", () => {
  const matches = { matches: false }
  vi.stubGlobal("matchMedia", () => matches)
  const skipTransition = vi.fn()
  const start = vi.fn(({ update }: { update: () => void }) => {
    update()
    return { skipTransition, finished: new Promise(() => {}) }
  })
  vi.stubGlobal("document", { startViewTransition: start })
  const update = vi.fn()
  move("open", update)
  move("swap", update)
  expect(skipTransition).toHaveBeenCalledOnce()
  expect(start).toHaveBeenLastCalledWith(
    expect.objectContaining({ types: ["swap"] })
  )
  matches.matches = true
  move("close", update)
  expect(start).toHaveBeenCalledTimes(2)
  expect(update).toHaveBeenCalledTimes(3)
})
