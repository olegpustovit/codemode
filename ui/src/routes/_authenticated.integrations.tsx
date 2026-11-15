import { createFileRoute } from "@tanstack/react-router"
import { z } from "zod"
import { IntegrationsPage } from "@/features/integrations"
import { integrationsQuery } from "@/features/queries"
export const Route = createFileRoute("/_authenticated/integrations")({
  validateSearch: z.object({
    q: z.string().max(1024).optional().catch(undefined),
    tab: z.enum(["all", "linked", "attention"]).optional().catch(undefined),
    selected: z.string().max(128).optional().catch(undefined),
    detail: z.literal("open").optional().catch(undefined),
    oauthError: z.string().max(2048).optional().catch(undefined),
    oauth: z.enum(["linked", "denied"]).optional().catch(undefined),
  }),
  loader: ({ context }) =>
    context.queryClient.query({ ...integrationsQuery, staleTime: "static" }),
  head: () => ({ meta: [{ title: "Integrations · Codemode" }] }),
  component: IntegrationsPage,
})
