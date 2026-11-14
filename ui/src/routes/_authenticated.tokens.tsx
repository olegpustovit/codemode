import { createFileRoute } from "@tanstack/react-router"
import { TokensPage } from "@/features/tokens"
import { tokensQuery, workspacesQuery } from "@/features/queries"
export const Route = createFileRoute("/_authenticated/tokens")({
  loader: ({ context }) =>
    Promise.all([
      context.queryClient.query({ ...tokensQuery, staleTime: "static" }),
      context.queryClient.query({ ...workspacesQuery, staleTime: "static" }),
    ]),
  head: () => ({ meta: [{ title: "Tokens · Codemode" }] }),
  component: TokensPage,
})
