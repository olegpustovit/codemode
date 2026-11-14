import { createFileRoute } from "@tanstack/react-router"
import { ApprovalPage, ApprovalRouteError } from "@/features/approvals"
import { approvalQuery } from "@/features/queries"
export const Route = createFileRoute("/_authenticated/approvals/$executionId")({
  loader: ({ context, params }) =>
    context.queryClient.query({
      ...approvalQuery(params.executionId),
      staleTime: "static",
    }),
  head: () => ({ meta: [{ title: "Review approval · Codemode" }] }),
  component: ApprovalPage,
  errorComponent: ApprovalRouteError,
})
