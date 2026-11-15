import { createFileRoute, redirect } from "@tanstack/react-router"
import { z } from "zod"
export const Route = createFileRoute("/_authenticated/sources/")({
  validateSearch: z.object({ q: z.string().optional().catch(undefined) }),
  beforeLoad: ({ search }) => {
    throw redirect({ to: "/integrations", search: { q: search.q } })
  },
})
