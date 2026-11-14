import { createFileRoute } from "@tanstack/react-router"
import { ScriptsPage } from "@/features/scripts"
import { workspaceQuery, scriptsQuery } from "@/features/queries"
export const Route = createFileRoute(
  "/_authenticated/workspaces/$workspaceId_/scripts/"
)({
  loader: ({ context, params }) =>
    Promise.all([
      context.queryClient.query({
        ...workspaceQuery(params.workspaceId),
        staleTime: "static",
      }),
      context.queryClient.query({
        ...scriptsQuery(params.workspaceId),
        staleTime: "static",
      }),
    ]),
  head: () => ({ meta: [{ title: "Saved scripts · Codemode" }] }),
  component: Page,
})
function Page() {
  const { workspaceId } = Route.useParams()
  return <ScriptsPage workspaceId={workspaceId} />
}
