import { createFileRoute, redirect } from "@tanstack/react-router"
export const Route = createFileRoute("/_authenticated/endpoints/$")({
  beforeLoad: ({ params }) => {
    throw redirect({ href: `/workspaces/${params._splat ?? ""}` })
  },
})
