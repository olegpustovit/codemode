import { afterEach, beforeEach, expect, test, vi } from "vitest"
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { ConfirmAction } from "./confirm-action"
import { StatusBar, StatusProvider, useStatus } from "./status-bar"

beforeEach(() => vi.useFakeTimers())
afterEach(() => {
  cleanup()
  vi.useRealTimers()
})
function Notice() {
  const { report } = useStatus()
  return <button onClick={() => report("URL copied")}>Copy</button>
}
function setup(remove = vi.fn(async () => {}), other = vi.fn(async () => {})) {
  const client = new QueryClient({
    defaultOptions: { mutations: { retry: false } },
  })
  const view = render(
    <QueryClientProvider client={client}>
      <StatusProvider>
        <ConfirmAction
          label="Delete source"
          description="This removes the connection."
          onConfirm={remove}
        />
        <ConfirmAction
          label="Revoke token"
          description="The agent loses access."
          onConfirm={other}
        />
        <Notice />
        <StatusBar />
      </StatusProvider>
    </QueryClientProvider>
  )
  return { ...view, remove, other }
}
async function settle() {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(0)
  })
}
test("requires two presses within three seconds and does not open a dialog", async () => {
  const { remove } = setup()
  fireEvent.click(screen.getByRole("button", { name: "Delete source" }))
  expect(remove).not.toHaveBeenCalled()
  expect(screen.queryByRole("dialog")).toBeNull()
  expect(screen.getByRole("status").textContent).toContain("within 3 seconds")
  fireEvent.click(screen.getByRole("button", { name: "Delete source" }))
  await settle()
  expect(remove).toHaveBeenCalledTimes(1)
  expect(screen.getByRole("status").textContent).toBe(
    "Completed: Delete source"
  )
})

test("expiry disarms deletion; a late click only starts a new confirmation", async () => {
  const { remove } = setup()
  fireEvent.click(screen.getByRole("button", { name: "Delete source" }))
  await act(async () => {
    await vi.advanceTimersByTimeAsync(3_000)
  })
  expect(screen.getByRole("status").textContent).toBe("Ready")
  fireEvent.click(screen.getByRole("button", { name: "Delete source" }))
  await settle()
  expect(remove).not.toHaveBeenCalled()
  fireEvent.click(screen.getByRole("button", { name: "Delete source" }))
  await settle()
  expect(remove).toHaveBeenCalledTimes(1)
})
test("arming another action invalidates the first, and the status prompt is clickable", async () => {
  const { remove, other } = setup()
  fireEvent.click(screen.getByRole("button", { name: "Delete source" }))
  fireEvent.click(screen.getByRole("button", { name: "Revoke token" }))
  fireEvent.click(screen.getByRole("button", { name: "Delete source" }))
  await settle()
  expect(remove).not.toHaveBeenCalled()
  expect(other).not.toHaveBeenCalled()
  fireEvent.click(
    screen.getByRole("button", { name: /Press Delete source again/ })
  )
  await settle()
  expect(remove).toHaveBeenCalledTimes(1)
  expect(other).not.toHaveBeenCalled()
})
test("an old confirmation timer cannot erase a newer status result", async () => {
  setup()
  fireEvent.click(screen.getByRole("button", { name: "Delete source" }))
  fireEvent.click(screen.getByRole("button", { name: "Copy" }))
  await act(async () => {
    await vi.advanceTimersByTimeAsync(3_000)
  })
  expect(screen.getByRole("status").textContent).toBe("URL copied")
})
test("failed operations show the actionable error in the status bar", async () => {
  const remove = vi.fn(async () => {
    throw new Error("Source changed; reload before deleting")
  })
  setup(remove)
  fireEvent.click(screen.getByRole("button", { name: "Delete source" }))
  fireEvent.click(screen.getByRole("button", { name: "Delete source" }))
  await settle()
  const status = screen.getByRole("status")
  expect(status.textContent).toContain("Source changed; reload before deleting")
  expect(status.getAttribute("data-tone")).toBe("error")
})
