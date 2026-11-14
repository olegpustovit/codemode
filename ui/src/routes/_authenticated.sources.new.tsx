import { createFileRoute } from "@tanstack/react-router"
import { NewSourcePage } from "@/features/sources"
export const Route = createFileRoute("/_authenticated/sources/new")({
  head: () => ({ meta: [{ title: "Sources · Codemode" }] }),
  component: NewSourcePage,
})
