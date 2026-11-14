import { createFileRoute } from "@tanstack/react-router"
import { WorkspacesPage } from "@/features/workspaces"
import { workspacesQuery } from "@/features/queries"
export const Route = createFileRoute("/_authenticated/workspaces/")({
  loader: ({ context }) =>
    context.queryClient.query({ ...workspacesQuery, staleTime: "static" }),
  head: () => ({ meta: [{ title: "Workspaces · Codemode" }] }),
  component: WorkspacesPage,
})
