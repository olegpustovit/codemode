import { createFileRoute } from "@tanstack/react-router"
import { NewWorkspacePage } from "@/features/workspaces"
import { sourcesQuery } from "@/features/queries"
export const Route = createFileRoute("/_authenticated/workspaces/new")({
  loader: ({ context }) =>
    context.queryClient.query({ ...sourcesQuery, staleTime: "static" }),
  head: () => ({ meta: [{ title: "Workspaces · Codemode" }] }),
  component: NewWorkspacePage,
})
