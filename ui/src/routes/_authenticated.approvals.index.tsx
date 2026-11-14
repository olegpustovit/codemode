import { createFileRoute } from "@tanstack/react-router"
import { ApprovalsPage } from "@/features/approvals"
import { approvalsQuery } from "@/features/queries"
export const Route = createFileRoute("/_authenticated/approvals/")({
  loader: ({ context }) =>
    context.queryClient.query({ ...approvalsQuery, staleTime: "static" }),
  head: () => ({ meta: [{ title: "Approvals · Codemode" }] }),
  component: ApprovalsPage,
})
