import { createFileRoute, redirect } from "@tanstack/react-router"
import { executionQuery } from "@/features/queries"

export const Route = createFileRoute("/_authenticated/executions/$executionId")(
  {
    loader: async ({ context, params }) => {
      const execution = await context.queryClient.ensureQueryData(
        executionQuery(params.executionId)
      )
      throw redirect({
        to: "/sessions",
        search: execution.sessionId
          ? { selected: execution.sessionId, execution: execution.id }
          : {},
        replace: true,
      })
    },
  }
)
