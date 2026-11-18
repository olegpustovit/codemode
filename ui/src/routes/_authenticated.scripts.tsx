import { createFileRoute } from "@tanstack/react-router"
import { z } from "zod"
import { ScriptLibraryPage } from "@/features/script-library"
export const Route = createFileRoute("/_authenticated/scripts")({
  validateSearch: z.object({
    q: z.string().max(1024).optional().catch(undefined),
    selected: z.uuid().optional().catch(undefined),
    detail: z.literal("open").optional().catch(undefined),
  }),
  head: () => ({ meta: [{ title: "Scripts · Codemode" }] }),
  component: ScriptLibraryPage,
})
