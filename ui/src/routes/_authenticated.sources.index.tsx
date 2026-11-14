import { createFileRoute } from "@tanstack/react-router"
import { SourcesPage } from "@/features/sources"
import { sourcesQuery } from "@/features/queries"
import { z } from "zod"
export const Route = createFileRoute("/_authenticated/sources/")({
  validateSearch: z.object({ q: z.string().optional().catch(undefined) }),
  loader: ({ context }) =>
    context.queryClient.query({ ...sourcesQuery, staleTime: "static" }),
  head: () => ({ meta: [{ title: "Sources · Codemode" }] }),
  component: SourcesPage,
})
