import { createFileRoute } from "@tanstack/react-router"
import { AuthorizePage } from "@/features/authorize"
import {
  authorizationSearch,
  consentQuery,
} from "@/features/authorization-query"
import { workspacesQuery, sourcesQuery } from "@/features/queries"
export const Route = createFileRoute("/_authenticated/authorize")({
  validateSearch: authorizationSearch,
  loaderDeps: ({ search }) => search,
  loader: ({ context, deps }) =>
    Promise.all([
      context.queryClient.query(consentQuery(deps)),
      context.queryClient.query({ ...workspacesQuery, staleTime: "static" }),
      context.queryClient.query({ ...sourcesQuery, staleTime: "static" }),
    ]),
  head: () => ({ meta: [{ title: "Authorize client · Codemode" }] }),
  component: AuthorizePage,
})
