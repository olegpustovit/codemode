import { createFileRoute } from "@tanstack/react-router"
import { z } from "zod"
import { WorkspaceToolsPage } from "@/features/workspace-tools"
import { workspacesQuery } from "@/features/queries"
export const Route = createFileRoute("/_authenticated/workspaces/")({
  validateSearch: z.object({
    q: z.string().max(1024).optional().catch(undefined),
    after: z.string().max(1024).optional().catch(undefined),
    selected: z.string().max(1024).optional().catch(undefined),
    detail: z.literal("open").optional().catch(undefined),
  }),
  loader: ({ context }) =>
    context.queryClient.query({ ...workspacesQuery, staleTime: "static" }),
  head: () => ({ meta: [{ title: "Workspaces · Codemode" }] }),
  component: WorkspaceToolsPage,
})
