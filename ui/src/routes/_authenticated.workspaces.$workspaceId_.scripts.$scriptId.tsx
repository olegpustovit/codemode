import { createFileRoute } from "@tanstack/react-router"
import { ScriptPage } from "@/features/scripts"
import { workspaceQuery, scriptQuery } from "@/features/queries"
export const Route = createFileRoute(
  "/_authenticated/workspaces/$workspaceId_/scripts/$scriptId"
)({
  loader: ({ context, params }) =>
    Promise.all([
      context.queryClient.query({
        ...workspaceQuery(params.workspaceId),
        staleTime: "static",
      }),
      context.queryClient.query({
        ...scriptQuery(params.workspaceId, params.scriptId),
        staleTime: "static",
      }),
    ]),
  head: () => ({ meta: [{ title: "Saved scripts · Codemode" }] }),
  component: Page,
})
function Page() {
  const { workspaceId, scriptId } = Route.useParams()
  return <ScriptPage workspaceId={workspaceId} scriptId={scriptId} />
}
