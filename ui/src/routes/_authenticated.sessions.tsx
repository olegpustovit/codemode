import { createFileRoute } from "@tanstack/react-router"
import { z } from "zod"
import { SessionsPage } from "@/features/sessions"
import { agentSessionsQuery } from "@/features/queries"
export const Route = createFileRoute("/_authenticated/sessions")({
  validateSearch: z.object({
    q: z.string().max(1024).optional().catch(undefined),
    view: z.enum(["list", "timeline"]).optional().catch(undefined),
    tab: z.enum(["all", "errors"]).optional().catch(undefined),
    cursor: z.uuid().optional().catch(undefined),
    runCursor: z.uuid().optional().catch(undefined),
    selected: z.uuid().optional().catch(undefined),
    execution: z.uuid().optional().catch(undefined),
    before: z.uuid().optional().catch(undefined),
    runBefore: z.uuid().optional().catch(undefined),
  }),
  loaderDeps: ({ search }) => ({ q: search.q, before: search.before }),
  loader: ({ context, deps }) =>
    context.queryClient.ensureQueryData(
      agentSessionsQuery(deps.q, deps.before)
    ),
  head: () => ({ meta: [{ title: "Sessions · Codemode" }] }),
  component: SessionsPage,
})
