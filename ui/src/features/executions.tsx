import type { ReactNode } from "react"
import { PythonCode } from "@/components/python-code"
import { Link, getRouteApi } from "@tanstack/react-router"
import { useSuspenseQuery } from "@tanstack/react-query"
import { useStatus } from "@/components/status-bar"
import { Button } from "@/components/ui/button"
import { ArrowLeft } from "lucide-react"
import type { ExecutionDetail } from "./queries"
import { executionQuery, executionsQuery } from "./queries"
import { PageHeader, Blank, StateBadge, date } from "@/components/common"
import {
  Table,
  TableHeader,
  TableHead,
  TableBody,
  TableRow,
  TableCell,
} from "@/components/ui/table"
import { Alert, AlertTitle, AlertDescription } from "@/components/ui/alert"
const listRoute = getRouteApi("/_authenticated/executions/")
const detailRoute = getRouteApi("/_authenticated/executions/$executionId")
export function ExecutionsPage() {
  const { before } = listRoute.useSearch()
  const executions = useSuspenseQuery(executionsQuery(before)).data
  return (
    <>
      <PageHeader
        title="Executions"
        description="Review code, tool calls and outcomes from your agents."
      />
      {!executions.length ? (
        <Blank
          title="No executions here"
          description="Executions appear after an agent runs Python through this gateway."
        />
      ) : (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Execution</TableHead>
              <TableHead>Started</TableHead>
              <TableHead>Status</TableHead>
              <TableHead className="text-right">Tool calls</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {executions.map((execution) => (
              <TableRow key={execution.id}>
                <TableCell>
                  <Link
                    className="font-mono text-xs underline underline-offset-4"
                    to="/executions/$executionId"
                    params={{ executionId: execution.id }}
                    aria-label={`Execution ${execution.id.slice(0, 8)}`}
                  >
                    {execution.id.slice(0, 8)}
                  </Link>
                </TableCell>
                <TableCell>{date(execution.startedAt)}</TableCell>
                <TableCell>
                  <StateBadge state={execution.status} />
                </TableCell>
                <TableCell className="text-right tabular-nums">
                  {execution.toolCalls}
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      )}
      <div className="flex justify-between gap-3">
        <Button
          variant="outline"
          disabled={!before}
          render={<Link to="/executions" search={{}} />}
        >
          Latest executions
        </Button>
        {executions.length === 50 && (
          <Button
            variant="outline"
            render={
              <Link
                to="/executions"
                search={{ before: executions.at(-1)?.id }}
              />
            }
          >
            Older executions
          </Button>
        )}
      </div>
    </>
  )
}
export function ExecutionPage() {
  const { executionId } = detailRoute.useParams()
  const execution = useSuspenseQuery(executionQuery(executionId)).data
  return <ExecutionContent execution={execution} />
}
export function ExecutionContent({
  execution,
  embedded = false,
  action,
}: {
  execution: ExecutionDetail
  embedded?: boolean
  action?: ReactNode
}) {
  const executionId = execution.id
  const { report } = useStatus()
  return (
    <>
      {!embedded && (
        <Link
          to="/executions"
          search={{}}
          className="flex items-center gap-2 text-sm text-muted-foreground"
        >
          <ArrowLeft className="size-4" />
          Executions
        </Link>
      )}
      {embedded ? (
        <header>
          <h2>{execution.title ?? "Python execution"}</h2>
          <StateBadge state={execution.status} />
        </header>
      ) : (
        <PageHeader
          title={`Execution ${executionId.slice(0, 8)}`}
          description={`Started ${date(execution.startedAt)}`}
          action={<StateBadge state={execution.status} />}
        />
      )}
      {execution.error && (
        <Alert variant="destructive">
          <AlertTitle>Execution error</AlertTitle>
          <AlertDescription>{execution.error}</AlertDescription>
        </Alert>
      )}
      <section className="flex flex-col gap-3">
        <h2>Python code</h2>
        <pre className="code-panel">
          <PythonCode code={execution.code} />
        </pre>
      </section>
      {execution.result !== null && (
        <section className="flex flex-col gap-3">
          <h2>Result preview</h2>
          <pre className="code-panel">
            <code>{execution.result}</code>
          </pre>
        </section>
      )}
      {execution.output !== null && (
        <section className="flex flex-col gap-3">
          <h2>Stdout</h2>
          <pre className="code-panel">
            <code>{execution.output || "No output"}</code>
          </pre>
        </section>
      )}
      <section className="flex flex-col gap-5">
        <h2>Tool calls</h2>
        {execution.calls.length === 0 ? (
          <p className="text-muted-foreground">
            This execution has no recorded tool calls.
          </p>
        ) : (
          execution.calls.map((call) => (
            <article
              key={call.seq}
              className="flex flex-col gap-3 border-b pb-5"
            >
              <div className="flex flex-wrap items-center justify-between gap-3">
                <h3 className="font-mono text-sm font-medium">
                  {call.seq + 1}. {call.path}
                </h3>
                <div className="flex items-center gap-3 text-sm">
                  <span className="text-muted-foreground tabular-nums">
                    {call.durationMs} ms
                  </span>
                  <StateBadge state={call.outcome} />
                </div>
              </div>
              <pre className="code-panel">
                <code>{JSON.stringify(call.argsRedacted, null, 2)}</code>
              </pre>
              {call.preview !== null && (
                <>
                  <h4>Result preview</h4>
                  <pre className="code-panel">
                    <code>{call.preview}</code>
                  </pre>
                </>
              )}
              {call.error && (
                <p className="text-sm break-words text-destructive">
                  {call.error}
                </p>
              )}
            </article>
          ))
        )}
      </section>
      {embedded && (
        <div className="detail-actions">
          {" "}
          <Button
            variant="outline"
            onClick={() =>
              void navigator.clipboard
                .writeText(execution.code)
                .then(() => report("Code copied"))
                .catch(() =>
                  report("Copy failed. Select the code and copy it.", "error")
                )
            }
          >
            Copy code
          </Button>
          {action}
        </div>
      )}
    </>
  )
}
