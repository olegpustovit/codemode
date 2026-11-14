import { createFileRoute } from "@tanstack/react-router"
import { AccountsPage } from "@/features/accounts"
import { sourcesQuery, accountsQuery } from "@/features/queries"
export const Route = createFileRoute("/_authenticated/accounts")({
  loader: ({ context }) =>
    Promise.all([
      context.queryClient.query({ ...sourcesQuery, staleTime: "static" }),
      context.queryClient.query({ ...accountsQuery, staleTime: "static" }),
    ]),
  head: () => ({ meta: [{ title: "Accounts · Codemode" }] }),
  component: AccountsPage,
})
