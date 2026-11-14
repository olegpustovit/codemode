import { createFileRoute } from "@tanstack/react-router"
import { ExecutionsPage } from "@/features/executions"
import { executionsQuery } from "@/features/queries"
import { z } from "zod"
export const Route = createFileRoute("/_authenticated/executions/")({
  validateSearch: z.object({ before: z.uuid().optional().catch(undefined) }),
  loaderDeps: ({ search }) => ({ before: search.before }),
  loader: ({ context, deps }) =>
    context.queryClient.query({
      ...executionsQuery(deps.before),
      staleTime: "static",
    }),
  head: () => ({ meta: [{ title: "Executions · Codemode" }] }),
  component: ExecutionsPage,
})
