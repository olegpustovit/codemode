import { Link, getRouteApi } from "@tanstack/react-router"
import {
  useMutation,
  useQueryClient,
  useSuspenseQuery,
} from "@tanstack/react-query"
import { ArrowLeft } from "lucide-react"
import { approvalsQuery, approvalQuery } from "./queries"
import { ApiError, nothing, write } from "@/lib/api"
import {
  Blank,
  ErrorNotice,
  PageHeader,
  RouteError,
  StateBadge,
  date,
} from "@/components/common"
import { Button } from "@/components/ui/button"
import {
  Table,
  TableHeader,
  TableHead,
  TableBody,
  TableRow,
  TableCell,
} from "@/components/ui/table"
import { Alert, AlertTitle, AlertDescription } from "@/components/ui/alert"
const detailRoute = getRouteApi("/_authenticated/approvals/$executionId")
export function ApprovalsPage() {
  const approvals = useSuspenseQuery(approvalsQuery).data
  return (
    <>
      <PageHeader
        title="Approvals"
        description="Review pending tool calls before an agent can continue."
      />
      {!approvals.length ? (
        <Blank
          title="No approvals waiting"
          description="Calls that require your permission appear here. An agent cannot approve its own calls."
        />
      ) : (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Execution</TableHead>
              <TableHead>Workspace</TableHead>
              <TableHead>Expires</TableHead>
              <TableHead className="text-right">Undecided calls</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {approvals.map((approval) => (
              <TableRow key={approval.executionId}>
                <TableCell>
                  <Link
                    className="font-mono text-xs underline underline-offset-4"
                    to="/approvals/$executionId"
                    params={{ executionId: approval.executionId }}
                  >
                    Review {approval.executionId.slice(0, 8)}
                  </Link>
                </TableCell>
                <TableCell className="break-all">{approval.hostname}</TableCell>
                <TableCell>{date(approval.expiresAt)}</TableCell>
                <TableCell className="text-right tabular-nums">
                  {approval.pending}
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      )}
    </>
  )
}
export function ApprovalPage() {
  const { executionId } = detailRoute.useParams()
  const approval = useSuspenseQuery(approvalQuery(executionId)).data
  const cache = useQueryClient()
  const decide = useMutation({
    mutationFn: ({ callId, allow }: { callId: number; allow: boolean }) =>
      write(
        `/api/approvals/${executionId}`,
        "POST",
        { pauseId: approval.pauseId, decisions: [{ callId, allow }] },
        nothing
      ),
    onSuccess: async () => {
      await cache.invalidateQueries({ queryKey: ["approvals"] })
    },
  })
  const pending = approval.calls.filter((call) => call.decision === null).length
  return (
    <>
      <Link
        to="/approvals"
        className="flex items-center gap-2 text-sm text-muted-foreground"
      >
        <ArrowLeft className="size-4" />
        Approvals
      </Link>
      <PageHeader
        title={`Review ${executionId.slice(0, 8)}`}
        description={`${approval.hostname} · Expires ${date(approval.expiresAt)}`}
        action={
          <StateBadge
            state={pending ? "awaiting_approval" : "decisions_saved"}
          />
        }
      />
      <Alert>
        <AlertTitle>
          {pending ? "This batch is waiting" : "Decisions saved"}
        </AlertTitle>
        <AlertDescription>
          {pending
            ? "No calls in this batch have been sent. Review each call. Your decision is final for this pause."
            : "The agent can now call resume to continue from this pause. Earlier tool calls will not run again."}
        </AlertDescription>
      </Alert>
      <section className="flex flex-col gap-3">
        <h2>Python code</h2>
        <pre className="code-panel">
          <code>{approval.code}</code>
        </pre>
      </section>
      <section className="flex flex-col gap-5">
        <h2>Pending tool calls</h2>
        <ErrorNotice error={decide.error} />
        {approval.calls.map((call) => (
          <article
            key={`${approval.pauseId}-${call.callId}`}
            className="flex flex-col gap-3 border-b pb-5"
          >
            <div className="flex flex-wrap items-center justify-between gap-3">
              <h3 className="font-mono text-sm font-medium break-all">
                {call.path}
              </h3>
              <StateBadge
                state={
                  call.error
                    ? "blocked"
                    : call.decision === null
                      ? "undecided"
                      : call.decision
                        ? "approved"
                        : "denied"
                }
              />
            </div>
            <pre className="code-panel">
              <code>{JSON.stringify(call.args, null, 2)}</code>
            </pre>
            {call.error && (
              <p className="text-sm break-words text-destructive">
                {call.error}
              </p>
            )}
            {call.decision === null && (
              <div className="flex flex-wrap gap-3">
                <Button
                  disabled={decide.isPending}
                  onClick={() =>
                    decide.mutate({ callId: call.callId, allow: true })
                  }
                  aria-label={`Allow ${call.path}`}
                >
                  Allow this call
                </Button>
                <Button
                  variant="outline"
                  disabled={decide.isPending}
                  onClick={() =>
                    decide.mutate({ callId: call.callId, allow: false })
                  }
                  aria-label={`Deny ${call.path}`}
                >
                  Deny this call
                </Button>
              </div>
            )}
          </article>
        ))}
      </section>
      <Button
        variant="outline"
        render={<Link to="/executions/$executionId" params={{ executionId }} />}
      >
        View execution log
      </Button>
    </>
  )
}
export function ApprovalRouteError({ error }: { error: unknown }) {
  const { executionId } = detailRoute.useParams()
  if (error instanceof ApiError && [404, 410].includes(error.status))
    return (
      <Blank
        title="This approval is no longer available"
        description="The execution continued, the approval expired, or its caller lost access. Check the execution log for its current state."
      >
        <Button
          variant="outline"
          render={
            <Link to="/executions/$executionId" params={{ executionId }} />
          }
        >
          View execution log
        </Button>
      </Blank>
    )
  return <RouteError error={error} />
}
