import { createFileRoute } from "@tanstack/react-router"
import { ExecutionPage } from "@/features/executions"
import { executionQuery } from "@/features/queries"
export const Route = createFileRoute("/_authenticated/executions/$executionId")(
  {
    loader: ({ context, params }) =>
      context.queryClient.query({
        ...executionQuery(params.executionId),
        staleTime: "static",
      }),
    head: () => ({ meta: [{ title: "Executions · Codemode" }] }),
    component: ExecutionPage,
  }
)
