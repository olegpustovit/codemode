import { createFileRoute, redirect } from "@tanstack/react-router"
import { AppShell } from "@/components/app-shell"
import { NotFound, Pending, RouteError } from "@/components/common"
import { sessionQuery } from "@/lib/runtime"
import { safeReturnPath } from "@/lib/api"
export const Route = createFileRoute("/_authenticated")({
  beforeLoad: async ({ context, location }) => {
    const session = await context.queryClient.query(sessionQuery)
    if (!session.authenticated)
      throw redirect({
        to: "/login",
        search: { redirect: safeReturnPath(location.href) },
      })
  },
  component: AppShell,
  pendingComponent: Pending,
  errorComponent: RouteError,
  notFoundComponent: NotFound,
})
