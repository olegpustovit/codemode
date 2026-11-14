import { createFileRoute } from "@tanstack/react-router"
import { NewScriptPage } from "@/features/scripts"
import { workspaceQuery } from "@/features/queries"
export const Route = createFileRoute(
  "/_authenticated/workspaces/$workspaceId_/scripts/new"
)({
  loader: ({ context, params }) =>
    Promise.all(
      [workspaceQuery(params.workspaceId)].map((query) =>
        context.queryClient.query({ ...query, staleTime: "static" })
      )
    ),
  head: () => ({ meta: [{ title: "Saved scripts · Codemode" }] }),
  component: Page,
})
function Page() {
  const { workspaceId } = Route.useParams()
  return <NewScriptPage workspaceId={workspaceId} />
}
