import { cleanup, fireEvent, render } from "@testing-library/react"
import { afterEach, expect, test, vi } from "vitest"
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

test("Python lines preserve operators, multiline strings and tool paths", () => {
  const code =
    'a == b\nfr"""first\nreturn second"""\nawait tools.moneybag.entries_list.all()'
  const onTool = vi.fn()
  const { container, getByRole } = render(
    <pre>
      <PythonCode code={code} onTool={onTool} />
    </pre>
  )
  expect(container.textContent).toBe(code)
  expect(container.querySelectorAll(".ln")).toHaveLength(4)
  expect(
    [...container.querySelectorAll(".code-string")].map(
      (node) => node.textContent
    )
  ).toEqual(['fr"""first', 'return second"""'])
  fireEvent.click(
    getByRole("button", { name: "tools.moneybag.entries_list.all" })
  )
  expect(onTool).toHaveBeenCalledWith("moneybag.entries_list")
})
