import { expect, test } from "vitest"
import { prettyJson } from "./pretty-json"

test("indents truncated JSON without dropping an unfinished escaped string", () => {
  expect(prettyJson(String.raw`{"x":[{"a":"quote\" and {, `)).toBe(
    '{\n  "x": [\n    {\n      "a": "quote\\" and {, '
  )
})
