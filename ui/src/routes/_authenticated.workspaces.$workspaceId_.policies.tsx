import { createFileRoute, redirect } from "@tanstack/react-router"
import { z } from "zod"
export const Route = createFileRoute(
  "/_authenticated/workspaces/$workspaceId_/policies"
)({
  validateSearch: z.object({
    q: z.string().max(1024).optional().catch(undefined),
    cursor: z.string().max(1024).optional().catch(undefined),
  }),
  beforeLoad: ({ params, search }) => {
    throw redirect({
      to: "/workspaces",
      search: { workspace: params.workspaceId, q: search.q },
    })
  },
  head: () => ({ meta: [{ title: "Tool policies · Codemode" }] }),
})
