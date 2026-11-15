import { afterEach, expect, test, vi } from "vitest"
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react"
import { BulkPane } from "./bulk-pane"
afterEach(cleanup)
test("destructive bulk actions confirm in the pane and focus the safe action", async () => {
  const apply = vi.fn(async () => {})
  render(
    <BulkPane
      count={3}
      names={["A", "B", "C"]}
      actions={[
        {
          id: "delete",
          label: "Delete scripts",
          confirm: "Delete these scripts? Recorded executions remain.",
        },
      ]}
      onAction={apply}
      onClose={() => {}}
    />
  )
  fireEvent.click(screen.getByRole("button", { name: "Delete scripts" }))
  expect(apply).not.toHaveBeenCalled()
  expect(document.activeElement).toBe(
    screen.getByRole("button", { name: /Keep selection/ })
  )
  expect(screen.queryByRole("dialog")).toBeNull()
  fireEvent.keyDown(document.activeElement!, { key: "Escape" })
  expect(
    screen.queryByText("Delete these scripts? Recorded executions remain.")
  ).toBeNull()
  fireEvent.click(screen.getByRole("button", { name: "Delete scripts" }))
  const destructive = screen.getByRole("button", { name: "Delete scripts" })
  destructive.focus()
  fireEvent.keyDown(destructive, { key: "Escape" })
  expect(screen.getByRole("heading", { name: "3 selected" })).toBeTruthy()
  expect(
    screen.queryByText("Delete these scripts? Recorded executions remain.")
  ).toBeNull()
  expect(apply).not.toHaveBeenCalled()
  fireEvent.click(screen.getByRole("button", { name: "Delete scripts" }))
  fireEvent.click(screen.getByRole("button", { name: "Delete scripts" }))
  await waitFor(() => expect(apply).toHaveBeenCalledOnce())
})
test("failed bulk changes retain the count and show a recovery error", async () => {
  render(
    <BulkPane
      count={300}
      allMatching
      names={["github.get_issue"]}
      actions={[{ id: "block", label: "Block" }]}
      onAction={async () => {
        throw new Error("Selection changed; reload and select again")
      }}
      onClose={() => {}}
    />
  )
  fireEvent.click(screen.getByRole("button", { name: "Block" }))
  await waitFor(() =>
    expect(screen.getByRole("alert").textContent).toContain("Selection changed")
  )
  expect(screen.getByRole("heading", { name: "300 selected" })).toBeTruthy()
})

test("confirmation keeps the original action snapshot when cached data refreshes", async () => {
  const original = vi.fn(async () => {}),
    refreshed = vi.fn(async () => {})
  const props = {
    count: 2,
    names: ["A", "B"],
    actions: [
      {
        id: "delete",
        label: "Delete",
        confirm: "Delete the selected revisions?",
      },
    ],
    onClose: () => {},
  }
  const view = render(<BulkPane {...props} onAction={original} />)
  fireEvent.click(screen.getByRole("button", { name: "Delete" }))
  view.rerender(<BulkPane {...props} onAction={refreshed} />)
  fireEvent.click(screen.getByRole("button", { name: "Delete" }))
  await waitFor(() => expect(original).toHaveBeenCalledOnce())
  expect(refreshed).not.toHaveBeenCalled()
})
