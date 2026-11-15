import { createFileRoute } from "@tanstack/react-router"
import { z } from "zod"
import { TokensPage } from "@/features/token-list"
import { tokensQuery, workspacesQuery } from "@/features/queries"
export const Route = createFileRoute("/_authenticated/tokens")({
  validateSearch: z.object({
    q: z.string().max(1024).optional().catch(undefined),
    selected: z.uuid().optional().catch(undefined),
    detail: z.enum(["open", "new"]).optional().catch(undefined),
  }),
  loader: ({ context }) =>
    Promise.all([
      context.queryClient.query({ ...tokensQuery, staleTime: "static" }),
      context.queryClient.query({ ...workspacesQuery, staleTime: "static" }),
    ]),
  head: () => ({ meta: [{ title: "Tokens · Codemode" }] }),
  component: TokensPage,
})
