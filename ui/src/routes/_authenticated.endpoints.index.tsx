import { createFileRoute, redirect } from "@tanstack/react-router"
export const Route = createFileRoute("/_authenticated/endpoints/")({
  beforeLoad: () => {
    throw redirect({ to: "/workspaces" })
  },
})
