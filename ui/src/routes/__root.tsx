import {
  createRootRouteWithContext,
  HeadContent,
  Outlet,
} from "@tanstack/react-router"
import type { QueryClient } from "@tanstack/react-query"
import { configQuery } from "@/lib/runtime"
import { NotFound, Pending, RouteError } from "@/components/common"
export const Route = createRootRouteWithContext<{ queryClient: QueryClient }>()(
  {
    loader: ({ context }) => context.queryClient.query(configQuery),
    head: () => ({ meta: [{ title: "Codemode" }] }),
    component: () => (
      <>
        <HeadContent />
        <Outlet />
      </>
    ),
    pendingComponent: Pending,
    errorComponent: RouteError,
    notFoundComponent: NotFound,
  }
)
