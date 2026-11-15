import { expect, test } from "vitest"
import { selectRows, selectAllRows } from "./use-list-navigation"
import type { ListSelection } from "./use-list-navigation"

const ids = ["a", "b", "c", "d", "e", "f", "g"]
const empty: ListSelection = { ids: [], allMatching: false }
test("range extension includes the cursor, shrinks, and works in both directions", () => {
  const range = selectRows(empty, ids, "a", "f", "range")
  expect(range.ids).toEqual(ids.slice(0, 6))
  expect(selectRows(range, ids, "f", "c", "range").ids).toEqual(["a", "b", "c"])
  const reverse = selectRows(empty, ids, "f", "b", "range")
  expect(reverse.ids).toEqual(["b", "c", "d", "e", "f"])
})
test("toggle adds and removes one, then establishes the next range anchor", () => {
  let selection = selectRows(empty, ids, "a", "b", "single")
  selection = selectRows(selection, ids, "b", "e", "toggle")
  expect(selection.ids).toEqual(["b", "e"])
  expect(selectRows(selection, ids, "e", "g", "range").ids).toEqual([
    "e",
    "f",
    "g",
  ])
  expect(selectRows(selection, ids, "e", "b", "toggle").ids).toEqual(["e"])
})
test("select all first covers loaded rows, then all matching; collapse clears the flag", () => {
  const loaded = selectAllRows(empty, ids)
  expect(loaded.ids).toEqual(ids)
  expect(loaded.allMatching).toBe(false)
  const matching = selectAllRows(loaded, ids)
  expect(matching.allMatching).toBe(true)
  expect(selectRows(matching, ids, "d", "d", "single")).toEqual({
    ids: ["d"],
    anchor: "d",
    allMatching: false,
  })
  expect(selectAllRows(empty, []).allMatching).toBe(false)
})
