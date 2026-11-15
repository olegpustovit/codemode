import { afterEach, beforeAll, expect, test, vi } from "vitest"
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import type { ComponentProps } from "react"
import { AccountMenu } from "./account-menu"

vi.mock("@tanstack/react-router", () => ({
  Link: ({ to, children, ...props }: ComponentProps<"a"> & { to: string }) => (
    <a href={to} {...props}>
      {children}
    </a>
  ),
}))
beforeAll(() => {
  vi.stubGlobal("PointerEvent", MouseEvent)
})
afterEach(cleanup)
test("opening the account menu preserves the session; signing out is deliberate", async () => {
  const user = userEvent.setup()
  const logout = vi.fn()
  render(<AccountMenu email="owner@example.com" onLogout={logout} />)
  fireEvent.mouseDown(
    screen.getByRole("button", { name: "Account menu for owner@example.com" }),
    { button: 0 }
  )
  expect(logout).not.toHaveBeenCalled()
  expect(
    (
      await screen.findByRole("menuitem", { name: "Account settings" })
    ).getAttribute("href")
  ).toBe("/account")
  expect(
    screen
      .getByRole("menuitem", { name: "Linked accounts" })
      .getAttribute("href")
  ).toBe("/accounts")
  await user.click(screen.getByRole("menuitem", { name: "Sign out" }))
  expect(logout).toHaveBeenCalledOnce()
})
test("Enter opens, arrows move, and Escape closes and returns focus", async () => {
  const user = userEvent.setup()
  const logout = vi.fn()
  render(<AccountMenu email="owner@example.com" onLogout={logout} />)
  const trigger = screen.getByRole("button", { name: /Account menu/ })
  trigger.focus()
  await user.keyboard("{Enter}")
  await waitFor(() =>
    expect(document.activeElement).toBe(
      screen.getByRole("menuitem", { name: "Account settings" })
    )
  )
  await user.keyboard("{ArrowDown}")
  expect(document.activeElement).toBe(
    screen.getByRole("menuitem", { name: "Linked accounts" })
  )
  await user.keyboard("{Escape}")
  await waitFor(() => expect(screen.queryByRole("menu")).toBeNull())
  expect(document.activeElement).toBe(trigger)
  expect(logout).not.toHaveBeenCalled()
})
