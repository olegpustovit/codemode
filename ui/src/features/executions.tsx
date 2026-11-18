import { useRef, useState, type ReactNode } from "react"
import { prettyJson } from "@/lib/pretty-json"
import { PythonCode } from "@/components/python-code"
import { useStatus } from "@/components/status-bar"
import { Button } from "@/components/ui/button"
import type { ExecutionDetail } from "./queries"
import { StateBadge, date } from "@/components/common"
import { Alert, AlertTitle, AlertDescription } from "@/components/ui/alert"

function CallPreview({ preview }: { preview: string }) {
  const [expanded, setExpanded] = useState(false)
  const lines = prettyJson(preview).split("\n")
  return (
    <>
      <pre className="trace-preview">
        <code>{(expanded ? lines : lines.slice(0, 24)).join("\n")}</code>
      </pre>
      {lines.length > 24 && (
        <button
          type="button"
          className="trace-more"
          aria-expanded={expanded}
          onClick={() => setExpanded(!expanded)}
        >
          {expanded ? "Show less" : "Show whole preview"}
        </button>
      )}
    </>
  )
}

function PrintedOutput({ output }: { output: string }) {
  const [raw, setRaw] = useState(false)
  let value: unknown
  try {
    value = JSON.parse(output)
  } catch {
    value = null
  }
  const entries =
    value && typeof value === "object" && !Array.isArray(value)
      ? Object.entries(value)
      : null
  const flat = entries?.every(
    ([, item]) => item === null || typeof item !== "object"
  )
  return (
    <>
      <h3>
        Printed{" "}
        {flat && (
          <button
            type="button"
            className="trace-more"
            aria-pressed={raw}
            onClick={() => setRaw(!raw)}
          >
            Raw output
          </button>
        )}
      </h3>
      {entries && flat && !raw ? (
        <table className="trace-output">
          <thead className="sr-only">
            <tr>
              <th>Key</th>
              <th>Value</th>
            </tr>
          </thead>
          <tbody>
            {entries.map(([key, item]) => (
              <tr key={key}>
                <th scope="row">{key}</th>
                <td>{JSON.stringify(item)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      ) : (
        <pre className="code-panel">
          <code>{output}</code>
        </pre>
      )}
    </>
  )
}

export function ExecutionContent({
  execution,
  action,
}: {
  execution: ExecutionDetail
  action?: ReactNode
}) {
  const callList = useRef<HTMLDivElement>(null)
  const totalToolTime = execution.calls.reduce(
    (total, call) => total + call.durationMs,
    0
  )
  const origin = Date.parse(execution.startedAt)
  const callOffsets = execution.calls.map((call) =>
    Math.max(0, Date.parse(call.startedAt) - origin)
  )
  const waterfallDuration = callOffsets.reduce(
    (end, offset, index) =>
      Math.max(end, offset + execution.calls[index].durationMs),
    1
  )
  function jumpToCall(path: string) {
    const call = Array.from(
      callList.current?.querySelectorAll("details") ?? []
    ).find((item) => item.dataset.path === path)
    if (!call) return
    call.open = true
    call.querySelector("summary")?.focus({ preventScroll: true })
    call.scrollIntoView({ block: "nearest" })
  }
  const { report } = useStatus()
  return (
    <>
      <header className="trace-header">
        <h2>{execution.title ?? "No tool calls"}</h2>
        <StateBadge state={execution.status} />
        <p>
          Started {date(execution.startedAt)} · {totalToolTime} ms ·{" "}
          {execution.calls.length} calls
        </p>
      </header>
      {execution.error && (
        <Alert variant="destructive">
          <AlertTitle>Execution error</AlertTitle>
          <AlertDescription>{execution.error}</AlertDescription>
        </Alert>
      )}
      <section className="trace-section">
        <h3>
          Tool calls{" "}
          <span>
            {execution.calls.length} calls · {totalToolTime} ms
          </span>
        </h3>
        <div className="trace-calls" ref={callList}>
          {execution.calls.length === 0 && (
            <p className="text-muted-foreground">No tool calls</p>
          )}
          {execution.calls.map((call, index) => {
            const args = Object.entries(call.argsRedacted ?? {})
            const left = (callOffsets[index] / waterfallDuration) * 100
            const width = (call.durationMs / waterfallDuration) * 100
            return (
              <details
                className="trace-call"
                data-path={call.path}
                key={call.seq}
              >
                <summary>
                  <span className="trace-index">{index + 1}</span>
                  <span className="trace-path" title={call.path}>
                    {call.path}
                  </span>
                  <span className="trace-args">
                    {args
                      .map(([key, value]) => `${key}=${JSON.stringify(value)}`)
                      .join(" ")}
                  </span>
                  <span className="trace-waterfall" aria-hidden="true">
                    <i style={{ left: `${left}%`, width: `${width}%` }} />
                  </span>
                  <span className="trace-duration">{call.durationMs} ms</span>
                  <StateBadge state={call.outcome} />
                </summary>
                <div className="trace-call-body">
                  <h4>Arguments</h4>
                  {args.length ? (
                    <dl className="trace-kv">
                      {args.map(([key, value]) => (
                        <div key={key}>
                          <dt>{key}</dt>
                          <dd>{JSON.stringify(value)}</dd>
                        </div>
                      ))}
                    </dl>
                  ) : (
                    <p className="text-muted-foreground">None</p>
                  )}
                  {call.preview !== null && (
                    <>
                      <h4>Result</h4>
                      <CallPreview preview={call.preview} />
                    </>
                  )}
                  {call.error && (
                    <p className="text-sm break-words text-destructive">
                      {call.error}
                    </p>
                  )}
                </div>
              </details>
            )
          })}
        </div>
      </section>
      <section className="trace-section">
        <h3>
          Python <span>{execution.code.split("\n").length} lines</span>
        </h3>
        <pre className="code-panel" tabIndex={0} aria-label="Python code">
          <PythonCode code={execution.code} onTool={jumpToCall} />
        </pre>
      </section>
      <section className="trace-section">
        <h3>Output</h3>
        <dl className="trace-return">
          <dt>Returned</dt>
          <dd>
            {execution.result === null ? (
              <span className="text-muted-foreground">nothing (None)</span>
            ) : (
              <code>{execution.result}</code>
            )}
          </dd>
        </dl>
        {execution.output && <PrintedOutput output={execution.output} />}
      </section>
      <div className="detail-actions">
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
    </>
  )
}
