import { createFileRoute, redirect } from "@tanstack/react-router"
export const Route = createFileRoute("/_authenticated/accounts")({
  beforeLoad: () => {
    throw redirect({ to: "/integrations", search: { tab: "linked" } })
  },
})
