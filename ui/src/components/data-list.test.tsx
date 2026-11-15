import { useState } from "react"
import { afterEach, expect, test, vi } from "vitest"
import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { DataList } from "./data-list"
import { TabBar } from "./tab-bar"
const rows = [
  { id: "a", label: "Alpha", group: "Development", cells: ["Alpha", "Linked"] },
  { id: "b", label: "Beta", group: "Knowledge", cells: ["Beta", "Needs link"] },
]
afterEach(cleanup)
test("one grid focus browses rows, clamps selection, opens with Enter and goes back", () => {
  const open = vi.fn(),
    back = vi.fn()
  function List() {
    const [selected, setSelected] = useState("a")
    return (
      <DataList
        rows={rows}
        columns={["Name", "State"]}
        label="Connections"
        selectedId={selected}
        onSelect={setSelected}
        onOpen={open}
        onBack={back}
      />
    )
  }
  const view = render(<List />)
  const grid = screen.getByRole("grid", { name: "Connections" })
  grid.focus()
  fireEvent.keyDown(grid, { key: "ArrowDown" })
  expect(
    screen.getByRole("row", { name: "Beta" }).getAttribute("aria-selected")
  ).toBe("true")
  expect(
    document
      .getElementById(grid.getAttribute("aria-activedescendant")!)
      ?.getAttribute("aria-label")
  ).toBe("Beta")
  fireEvent.keyDown(grid, { key: "ArrowDown" })
  fireEvent.keyDown(grid, { key: "Enter" })
  expect(open).toHaveBeenLastCalledWith("b")
  fireEvent.keyDown(grid, { key: "Home" })
  expect(
    screen.getByRole("row", { name: "Alpha" }).getAttribute("aria-selected")
  ).toBe("true")
  fireEvent.keyDown(grid, { key: "Escape" })
  expect(back).toHaveBeenCalledOnce()
  fireEvent.click(screen.getByRole("row", { name: "Beta" }))
  expect(open).toHaveBeenLastCalledWith("b")
  expect(document.activeElement).toBe(grid)
  expect(view.container.querySelectorAll(".data-cursor").length).toBe(1)
})
test("empty lists have no active descendant and do not invoke actions", () => {
  const open = vi.fn(),
    select = vi.fn()
  render(
    <DataList
      rows={[]}
      columns={["Name"]}
      label="Empty"
      onSelect={select}
      onOpen={open}
      onBack={() => {}}
    />
  )
  const grid = screen.getByRole("grid")
  fireEvent.keyDown(grid, { key: "ArrowDown" })
  fireEvent.keyDown(grid, { key: "Enter" })
  expect(open).not.toHaveBeenCalled()
  expect(select).not.toHaveBeenCalled()
  expect(grid.hasAttribute("aria-activedescendant")).toBe(false)
  expect(screen.queryByRole("columnheader")).toBeNull()
  expect(screen.getByText("No empty")).toBeTruthy()
})
test("empty lists expose the supplied recovery actions without headers or cursor", () => {
  const view = render(
    <DataList
      rows={[]}
      columns={["Script"]}
      label="Scripts"
      onSelect={() => {}}
      onOpen={() => {}}
      onBack={() => {}}
      empty={
        <>
          <h2>No saved scripts</h2>
          <button>New script</button>
          <a href="/sessions">Open Sessions</a>
        </>
      }
    />
  )
  expect(screen.getByRole("button", { name: "New script" })).toBeTruthy()
  expect(
    screen.getByRole("link", { name: "Open Sessions" }).getAttribute("href")
  ).toBe("/sessions")
  expect(screen.queryByRole("columnheader")).toBeNull()
  expect(view.container.querySelector(".data-cursor")).toBeNull()
})
test("range and modifier selection remain local, and Escape collapses before closing", () => {
  const open = vi.fn(),
    back = vi.fn()
  function List() {
    const [cursor, setCursor] = useState("a")
    return (
      <DataList
        rows={rows}
        columns={["Name", "State"]}
        label="Connections"
        selectedId={cursor}
        onSelect={setCursor}
        onOpen={open}
        onBack={back}
        totalMatches={300}
      />
    )
  }
  render(<List />)
  const grid = screen.getByRole("grid")
  expect(grid.getAttribute("aria-multiselectable")).toBe("true")
  fireEvent.keyDown(grid, { key: "ArrowDown", shiftKey: true })
  expect(
    screen
      .getAllByRole("row")
      .filter((row) => row.getAttribute("aria-selected") === "true")
  ).toHaveLength(2)
  expect(open).not.toHaveBeenCalled()
  fireEvent.keyDown(grid, { key: "a", ctrlKey: true })
  expect(screen.getByText("300 selected")).toBeTruthy()
  fireEvent.keyDown(grid, { key: "Escape" })
  expect(back).not.toHaveBeenCalled()
  expect(
    screen
      .getAllByRole("row")
      .filter((row) => row.getAttribute("aria-selected") === "true")
  ).toHaveLength(1)
  fireEvent.keyDown(grid, { key: "Escape" })
  expect(back).toHaveBeenCalledOnce()
  fireEvent.click(screen.getByRole("row", { name: "Alpha" }), { metaKey: true })
  expect(open).not.toHaveBeenCalled()
  expect(screen.getByText("2 selected")).toBeTruthy()
})
test("End requests another page and F2 edits without activating the row", async () => {
  const load = vi.fn(),
    rename = vi.fn(async () => {}),
    open = vi.fn()
  render(
    <DataList
      rows={rows}
      columns={["Name", "State"]}
      label="Connections"
      onSelect={() => {}}
      onOpen={open}
      onBack={() => {}}
      onLoadMore={load}
      onRename={rename}
    />
  )
  const grid = screen.getByRole("grid")
  fireEvent.keyDown(grid, { key: "End" })
  expect(load).toHaveBeenCalledOnce()
  fireEvent.keyDown(grid, { key: "F2" })
  const input = screen.getByRole("textbox", { name: "Rename Alpha" })
  fireEvent.change(input, { target: { value: "ACME Billing" } })
  fireEvent.keyDown(input, { key: "Enter" })
  expect(rename).toHaveBeenCalledWith("a", "ACME Billing")
  expect(open).not.toHaveBeenCalled()
})
test("tab keyboard selection moves focus and shares one underline", () => {
  function Tabs() {
    const [selected, setSelected] = useState("all")
    return (
      <TabBar
        tabs={[
          { id: "all", label: "All", count: 2 },
          { id: "linked", label: "Linked", count: 1 },
        ]}
        selected={selected}
        onSelect={setSelected}
        panelId="results"
      />
    )
  }
  const view = render(<Tabs />)
  const first = screen.getByRole("tab", { name: "All 2" })
  first.focus()
  fireEvent.keyDown(first, { key: "ArrowRight" })
  const linked = screen.getByRole("tab", { name: "Linked 1" })
  expect(linked.getAttribute("aria-selected")).toBe("true")
  expect(document.activeElement).toBe(linked)
  expect(view.container.querySelectorAll(".tab-underline").length).toBe(1)
})
