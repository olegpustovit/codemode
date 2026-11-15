import { createFileRoute } from "@tanstack/react-router"
import { NewScriptPage } from "@/features/scripts"
import { z } from "zod"
import { useSuspenseQuery } from "@tanstack/react-query"
import { workspaceQuery, executionQuery } from "@/features/queries"
export const Route = createFileRoute(
  "/_authenticated/workspaces/$workspaceId_/scripts/new"
)({
  validateSearch: z.object({
    fromExecution: z.uuid().optional().catch(undefined),
  }),
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
  const { fromExecution } = Route.useSearch()
  return fromExecution ? (
    <FromExecution
      key={fromExecution}
      workspaceId={workspaceId}
      executionId={fromExecution}
    />
  ) : (
    <NewScriptPage workspaceId={workspaceId} />
  )
}

function FromExecution({
  workspaceId,
  executionId,
}: {
  workspaceId: string
  executionId: string
}) {
  const execution = useSuspenseQuery(executionQuery(executionId)).data
  return (
    <NewScriptPage workspaceId={workspaceId} initialCode={execution.code} />
  )
}
