import { Suspense } from "react"
import { afterEach, expect, test, vi } from "vitest"
import { act, cleanup, render, screen } from "@testing-library/react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { Route } from "./login"
import { configQuery, sessionQuery } from "@/lib/runtime"

vi.mock("@tanstack/react-router", async (original) => ({
  ...(await original<object>()),
  createFileRoute: () => (options: { component: unknown }) => ({
    ...options,
    useSearch: () => ({ redirect: "/sources?tab=a b" }),
  }),
  useRouter: () => ({ navigate: vi.fn() }),
}))
afterEach(cleanup)

async function show(idEnabled: boolean) {
  const client = new QueryClient()
  client.setQueryData(configQuery.queryKey, {
    name: "Codemode",
    hostname: "codemode.test",
    loginEnabled: !idEnabled,
    idEnabled,
  })
  client.setQueryData(sessionQuery.queryKey, {
    authenticated: false,
    firstUser: false,
    user: null,
  })
  const Login = (Route as unknown as { component: () => React.ReactNode })
    .component
  await act(async () => {
    render(
      <QueryClientProvider client={client}>
        <Suspense>
          <Login />
        </Suspense>
      </QueryClientProvider>
    )
  })
}

test("legacy mode keeps the password form", async () => {
  await show(false)
  expect(await screen.findByLabelText("Password")).toBeTruthy()
  expect(screen.queryByRole("link", { name: /Microapps ID/ })).toBeNull()
})

test("Microapps ID mode offers only the Microapps ID sign-in link with an encoded return path", async () => {
  await show(true)
  const link = await screen.findByRole("link", {
    name: "Sign in with Microapps ID",
  })
  expect(link.getAttribute("href")).toBe(
    "/auth/login?return_to=%2Fsources%3Ftab%3Da%20b"
  )
  expect(screen.queryByLabelText("Password")).toBeNull()
})
