import { QueryClient } from "@tanstack/react-query"
import { createRouter } from "@tanstack/react-router"
import { ApiError } from "@/lib/api"
import { routeTree } from "./routeTree.gen"
export const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 15_000,
      retry: (count, error) =>
        !(error instanceof ApiError && error.status < 500) && count < 2,
    },
    mutations: { retry: false, gcTime: 0 },
  },
})
export const router = createRouter({
  routeTree,
  context: { queryClient },
  defaultPreload: "intent",
  defaultPreloadStaleTime: 0,
  scrollRestoration: true,
})
declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router
  }
}
