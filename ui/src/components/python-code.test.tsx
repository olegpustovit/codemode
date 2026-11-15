import { cleanup, render } from "@testing-library/react"
import { afterEach, expect, test } from "vitest"
import { PythonCode } from "./python-code"
afterEach(cleanup)
test("Python colors preserve source and never interpret HTML or highlight inside strings", () => {
  const code =
    '# return a result\nreturn "<script>await # unsafe</script>"\nfor x in xs:\n  await x()\n"""return\n# still a string"""'
  const { container } = render(
    <pre>
      <PythonCode code={code} />
    </pre>
  )
  expect(container.textContent).toBe(code)
  expect(container.querySelector("script")).toBeNull()
  expect(
    [...container.querySelectorAll(".code-keyword")].map((e) => e.textContent)
  ).toEqual(["return", "for", "in", "await"])
  expect(
    [...container.querySelectorAll(".code-comment")].map((e) => e.textContent)
  ).toEqual(["# return a result"])
})
