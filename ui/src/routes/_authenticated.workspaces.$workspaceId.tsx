import { createFileRoute } from "@tanstack/react-router"
import { WorkspacePage } from "@/features/workspaces"
import { workspaceQuery, sourcesQuery } from "@/features/queries"
export const Route = createFileRoute("/_authenticated/workspaces/$workspaceId")(
  {
    loader: ({ context, params }) =>
      Promise.all([
        context.queryClient.query({
          ...workspaceQuery(params.workspaceId),
          staleTime: "static",
        }),
        context.queryClient.query({ ...sourcesQuery, staleTime: "static" }),
      ]),
    head: () => ({ meta: [{ title: "Workspaces · Codemode" }] }),
    component: WorkspacePage,
  }
)
