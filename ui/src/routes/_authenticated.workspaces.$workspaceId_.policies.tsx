import { createFileRoute } from "@tanstack/react-router"
import { z } from "zod"
import { PoliciesPage } from "@/features/policies"
import { workspaceQuery, policyQuery } from "@/features/queries"
export const Route = createFileRoute(
  "/_authenticated/workspaces/$workspaceId_/policies"
)({
  validateSearch: z.object({
    q: z.string().max(1024).optional().catch(undefined),
    cursor: z.string().max(1024).optional().catch(undefined),
  }),
  loaderDeps: ({ search }) => search,
  loader: ({ context, params, deps }) =>
    Promise.all([
      context.queryClient.query({
        ...workspaceQuery(params.workspaceId),
        staleTime: "static",
      }),
      context.queryClient.query({
        ...policyQuery(params.workspaceId, deps.q, deps.cursor),
        staleTime: "static",
      }),
    ]),
  head: () => ({ meta: [{ title: "Tool policies · Codemode" }] }),
  component: PoliciesPage,
})
