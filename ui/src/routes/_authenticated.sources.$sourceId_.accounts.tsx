import { createFileRoute } from "@tanstack/react-router"
import { AccountPage } from "@/features/accounts"
import { sourceQuery, accountsQuery } from "@/features/queries"
import { z } from "zod"
export const Route = createFileRoute(
  "/_authenticated/sources/$sourceId_/accounts"
)({
  validateSearch: z.object({
    oauth: z.enum(["linked", "denied"]).optional().catch(undefined),
  }),
  loader: ({ context, params }) =>
    Promise.all([
      context.queryClient.query({
        ...sourceQuery(params.sourceId),
        staleTime: "static",
      }),
      context.queryClient.query({ ...accountsQuery, staleTime: "static" }),
    ]),
  head: () => ({ meta: [{ title: "Accounts · Codemode" }] }),
  component: AccountPage,
})
