import { createFileRoute } from "@tanstack/react-router"
import { SourcePage } from "@/features/sources"
import { sourceQuery, workspacesQuery } from "@/features/queries"
export const Route = createFileRoute("/_authenticated/sources/$sourceId")({
  loader: ({ context, params }) =>
    Promise.all([
      context.queryClient.query({
        ...sourceQuery(params.sourceId),
        staleTime: "static",
      }),
      context.queryClient.query({ ...workspacesQuery, staleTime: "static" }),
    ]),
  head: () => ({ meta: [{ title: "Sources · Codemode" }] }),
  component: SourcePage,
})
