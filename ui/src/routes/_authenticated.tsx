import {
  createFileRoute,
  redirect,
  retainSearchParams,
} from "@tanstack/react-router"
import { AppShell } from "@/components/app-shell"
import { NotFound, Pending, RouteError } from "@/components/common"
import { sessionQuery } from "@/lib/runtime"
import { safeReturnPath } from "@/lib/api"
import { workspaceId } from "@/lib/workspace-url"
export const Route = createFileRoute("/_authenticated")({
  validateSearch: (
    search: Record<string, unknown>
  ): { workspace?: string } => ({
    workspace: workspaceId(search.workspace),
  }),
  search: { middlewares: [retainSearchParams(["workspace"])] },
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
