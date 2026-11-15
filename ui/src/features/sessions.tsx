import { Breadcrumbs } from "@/components/breadcrumbs"
import { useEffect, useMemo, useRef, useState } from "react"
import { Link, getRouteApi } from "@tanstack/react-router"
import {
  useInfiniteQuery,
  useMutation,
  useQuery,
  useQueryClient,
  useSuspenseQuery,
} from "@tanstack/react-query"
import { Search } from "lucide-react"
import type { AgentSession } from "./queries"
import {
  agentSessionsQuery,
  agentSessionQuery,
  executionQuery,
  sessionActivitySchema,
} from "./queries"
import { ExecutionContent } from "./executions"
import { DataList } from "@/components/data-list"
import { PaneStack } from "@/components/pane-stack"
import { TabBar } from "@/components/tab-bar"
import { ShellSlot } from "@/components/shell-slot"
import { useStatus } from "@/components/status-bar"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { ErrorNotice, date } from "@/components/common"
import { api, message, nothing, write } from "@/lib/api"
const route = getRouteApi("/_authenticated/sessions")
export function SessionsPage() {
  const search = route.useSearch()
  const navigate = route.useNavigate()
  const page = useSuspenseQuery(
    agentSessionsQuery(search.q, search.before)
  ).data
  const rows = page.sessions.filter(
    (s) => search.tab !== "errors" || s.failures > 0
  )
  const input = useRef<HTMLInputElement>(null)
  const list = useRef<HTMLTableElement>(null)
  const runs = useRef<HTMLTableElement>(null)
  const session = useQuery({
    ...agentSessionQuery(search.selected ?? "", search.runBefore),
    enabled: !!search.selected,
  })
  const execution = useQuery({
    ...executionQuery(search.execution ?? ""),
    enabled:
      !!search.execution &&
      session.data?.executions.some((e) => e.id === search.execution) === true,
  })
  const timeline = search.view === "timeline" && !search.execution
  const activity = useInfiniteQuery({
    queryKey: ["session-activity"],
    initialPageParam: "",
    queryFn: ({ pageParam }) =>
      api(
        `/api/session-activity${pageParam ? `?before=${pageParam}` : ""}`,
        sessionActivitySchema
      ),
    getNextPageParam: (page) => page.nextCursor ?? undefined,
    enabled: timeline,
    refetchInterval: 5_000,
  })
  const { hasNextPage, isFetching, isFetchNextPageError, fetchNextPage } =
    activity
  useEffect(() => {
    if (timeline && hasNextPage && !isFetching && !isFetchNextPageError)
      void fetchNextPage()
  }, [timeline, hasNextPage, isFetching, isFetchNextPageError, fetchNextPage])
  const ticks = useMemo(() => {
    const grouped = new Map<
      string,
      { id: string; startedAt: string; status: string }[]
    >()
    for (const page of activity.data?.pages ?? [])
      for (const tick of page.executions) {
        const rows = grouped.get(tick.sessionId) ?? []
        rows.push(tick)
        grouped.set(tick.sessionId, rows)
      }
    return grouped
  }, [activity.data])
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    const timer = setInterval(() => {
      if (document.visibilityState !== "hidden") setNow(Date.now())
    }, 5_000)
    return () => clearInterval(timer)
  }, [])
  const toggleView = () =>
    void navigate({
      search: {
        ...search,
        view: search.view === "timeline" ? "list" : "timeline",
      },
    })
  const selected = search.selected ?? search.cursor ?? rows[0]?.id
  const back = () => {
    if (search.execution) {
      void navigate({ search: { ...search, execution: undefined } })
      requestAnimationFrame(() => runs.current?.focus())
    } else {
      void navigate({
        search: {
          ...search,
          selected: undefined,
          q: search.q ? undefined : search.q,
          runBefore: undefined,
        },
      })
      requestAnimationFrame(() => list.current?.focus())
    }
  }
  const choose = (id: string, open = false) =>
    void navigate({
      search: {
        ...search,
        selected: id,
        execution: undefined,
        runBefore: undefined,
      },
      replace: !open,
    })
  useEffect(() => {
    const keys = (event: KeyboardEvent) => {
      if (
        event.defaultPrevented ||
        event.ctrlKey ||
        event.metaKey ||
        event.altKey
      )
        return
      if (
        event.target instanceof Element &&
        event.target.closest(
          "input,textarea,select,[contenteditable=true],#section-navigation"
        )
      )
        return
      if (event.key === "/") {
        event.preventDefault()
        input.current?.focus()
      } else if (event.key.toLowerCase() === "v") {
        event.preventDefault()
        toggleView()
      } else if (event.key === "[" || event.key === "]") {
        event.preventDefault()
        void navigate({
          search: {
            view: search.view,
            q: search.q,
            tab: search.tab === "errors" ? "all" : "errors",
          },
        })
      } else if (event.key === "Escape" || event.key === "ArrowLeft") {
        event.preventDefault()
        back()
      } else if (
        search.execution &&
        (event.key === "ArrowDown" || event.key === "ArrowUp")
      ) {
        const items = session.data?.executions ?? []
        const index = items.findIndex((e) => e.id === search.execution)
        const next =
          items[
            Math.max(
              0,
              Math.min(
                items.length - 1,
                index + (event.key === "ArrowDown" ? 1 : -1)
              )
            )
          ]
        if (next) {
          event.preventDefault()
          void navigate({
            search: { ...search, execution: next.id },
            replace: true,
          })
        }
      }
    }
    document.addEventListener("keydown", keys)
    return () => document.removeEventListener("keydown", keys)
  })
  return (
    <div className="list-screen sessions-screen">
      <ShellSlot target="shell-search">
        <label className="global-search" htmlFor="session-search">
          <Search aria-hidden="true" />
          <Input
            id="session-search"
            ref={input}
            type="search"
            aria-label="Search sessions"
            placeholder="Search sessions"
            value={search.q ?? ""}
            onChange={(e) =>
              void navigate({
                search: {
                  view: search.view,
                  q: e.target.value || undefined,
                  tab: search.tab,
                },
                replace: true,
              })
            }
            onKeyDown={(e) => {
              if (e.key === "Escape") {
                e.preventDefault()
                back()
              }
            }}
          />
          <span className="kbd">/</span>
        </label>
      </ShellSlot>
      <header className="list-heading">
        <div>
          <Breadcrumbs
            workspaceId={session.data?.session.workspaceId}
            items={[
              {
                label: "Sessions",
                onSelect: () =>
                  void navigate({
                    search: {
                      ...search,
                      selected: undefined,
                      execution: undefined,
                    },
                  }),
              },
              ...(session.data
                ? [
                    {
                      label: session.data.session.title,
                      onSelect: search.execution
                        ? () =>
                            void navigate({
                              search: { ...search, execution: undefined },
                            })
                        : undefined,
                    },
                  ]
                : []),
              ...(search.execution
                ? [{ label: execution.data?.title ?? "Execution" }]
                : []),
            ]}
          />
          <h1>Sessions</h1>
          <p>
            What your agents ran, grouped automatically. Kept for 4 days after
            the last activity.
          </p>
        </div>
      </header>
      <fieldset className="session-view-controls" aria-label="Sessions layout">
        <button
          aria-pressed={search.view !== "timeline"}
          onClick={() => void navigate({ search: { ...search, view: "list" } })}
        >
          List
        </button>
        <button
          aria-pressed={search.view === "timeline"}
          onClick={() =>
            void navigate({ search: { ...search, view: "timeline" } })
          }
        >
          Timeline
        </button>
        <span className="kbd">V</span>
      </fieldset>
      <TabBar
        label="Filter sessions"
        panelId="session-panes"
        tabs={[
          { id: "all", label: "All", count: page.sessions.length },
          {
            id: "errors",
            label: "With errors",
            count: page.sessions.filter((s) => s.failures > 0).length,
          },
        ]}
        selected={search.tab ?? "all"}
        onSelect={(tab) =>
          void navigate({
            search: {
              view: search.view,
              q: search.q,
              tab: tab === "errors" ? "errors" : "all",
            },
          })
        }
      />
      <PaneStack
        id="session-panes"
        label="Session"
        open={!!search.selected}
        list={
          <>
            <DataList
              label={timeline ? "Sessions timeline" : "Sessions"}
              className={timeline ? "timeline-list" : undefined}
              busy={timeline && activity.isFetching}
              listRef={list}
              columns={
                timeline
                  ? ["Session", <TimelineAxis key="axis" now={now} />]
                  : [
                      "Session",
                      "Agent",
                      "Runs",
                      "Calls",
                      "Status",
                      "Last activity",
                    ]
              }
              selectedId={selected}
              rows={rows.map((s) => ({
                id: s.id,
                label: s.title,
                group: new Date(s.lastSeenAt).toLocaleDateString(undefined, {
                  weekday: "long",
                  month: "short",
                  day: "numeric",
                }),
                cells: timeline
                  ? [
                      s.title,
                      <TimelineLane
                        key="track"
                        session={s}
                        ticks={ticks.get(s.id) ?? []}
                        now={now}
                      />,
                    ]
                  : [
                      s.title,
                      s.client,
                      s.executions,
                      s.toolCalls,
                      <span
                        key="status"
                        className={s.failures ? "bad" : undefined}
                      >
                        {s.status}
                      </span>,
                      date(s.lastSeenAt),
                    ],
              }))}
              onSelect={(id) => {
                if (search.selected) choose(id)
                else
                  void navigate({
                    search: { ...search, cursor: id },
                    replace: true,
                  })
              }}
              onOpen={(id) => choose(id, true)}
              onBack={back}
              empty={
                search.q
                  ? "No matching sessions. Press Esc to clear your search."
                  : "Waiting for the first execution…"
              }
            />
            {timeline && (
              <>
                <ErrorNotice error={activity.error} />
                {activity.isFetching && (
                  <p className="timeline-loading">Loading execution ticks…</p>
                )}
              </>
            )}
            <div className="pane-pagination">
              <Button
                variant="ghost"
                disabled={!search.before}
                onClick={() =>
                  void navigate({ search: { ...search, before: undefined } })
                }
              >
                Latest sessions
              </Button>
              <Button
                variant="ghost"
                disabled={!page.nextCursor}
                onClick={() =>
                  void navigate({
                    search: { ...search, before: page.nextCursor ?? undefined },
                  })
                }
              >
                Older sessions
              </Button>
            </div>
          </>
        }
        detail={
          <>
            <div className="pane-heading">
              <span>
                Session{session.data ? ` · ${session.data.session.client}` : ""}
              </span>
              <Button variant="ghost" onClick={back}>
                Close <span className="kbd">Esc</span>
              </Button>
            </div>
            <ErrorNotice error={session.error} />
            {session.isPending && <p>Loading session…</p>}
            {session.data && (
              <>
                <SessionFacts
                  key={session.data.session.id}
                  session={session.data.session}
                  now={now}
                />
                <DataList
                  label="Session executions"
                  listRef={runs}
                  columns={["Execution", "Calls", "Status"]}
                  selectedId={search.execution ?? search.runCursor}
                  rows={session.data.executions.map((e) => ({
                    id: e.id,
                    label: e.title ?? "Python execution",
                    group: "Executions",
                    cells: [
                      e.title ?? "Python execution",
                      e.toolCalls,
                      <span
                        key="status"
                        className={e.status === "failed" ? "bad" : undefined}
                      >
                        {e.status}
                      </span>,
                    ],
                  }))}
                  onSelect={(id) =>
                    void navigate({
                      search: {
                        ...search,
                        runCursor: id,
                        execution: search.execution ? id : undefined,
                      },
                      replace: true,
                    })
                  }
                  onOpen={(id) =>
                    void navigate({ search: { ...search, execution: id } })
                  }
                  onBack={back}
                />
                <div className="pane-pagination">
                  <Button
                    variant="ghost"
                    disabled={!search.runBefore}
                    onClick={() =>
                      void navigate({
                        search: {
                          ...search,
                          runBefore: undefined,
                          execution: undefined,
                        },
                      })
                    }
                  >
                    Latest executions
                  </Button>
                  <Button
                    variant="ghost"
                    disabled={!session.data.nextCursor}
                    onClick={() =>
                      void navigate({
                        search: {
                          ...search,
                          runBefore: session.data?.nextCursor ?? undefined,
                          execution: undefined,
                        },
                      })
                    }
                  >
                    Older executions
                  </Button>
                </div>
              </>
            )}
          </>
        }
        execution={
          search.execution ? (
            <>
              <div className="pane-heading">
                <span>Execution</span>
                <Button variant="ghost" onClick={back}>
                  Close <span className="kbd">Esc</span>
                </Button>
              </div>
              <ErrorNotice error={execution.error} />
              {execution.data && session.data ? (
                <div className="pane-body execution-content">
                  <ExecutionContent
                    execution={execution.data}
                    embedded
                    action={
                      <Button
                        variant="outline"
                        render={
                          <Link
                            to="/workspaces/$workspaceId/scripts/new"
                            params={{
                              workspaceId: session.data.session.workspaceId,
                            }}
                            search={{ fromExecution: execution.data.id }}
                          />
                        }
                      >
                        Save as script
                      </Button>
                    }
                  />
                </div>
              ) : (
                <p>
                  {execution.isFetching
                    ? "Loading execution…"
                    : "Execution is not in this session page. Close this pane and select an execution."}
                </p>
              )}
            </>
          ) : undefined
        }
      />
      <ShellSlot target="status-keys">
        <button
          onClick={() => selected && choose(selected, true)}
          disabled={!selected}
        >
          <span className="kbd">↵</span>Open
        </button>
        <button onClick={toggleView}>
          <span className="kbd">V</span>Layout
        </button>
        <button onClick={back}>
          <span className="kbd">Esc</span>Back
        </button>
        <button onClick={() => input.current?.focus()}>
          <span className="kbd">/</span>Search
        </button>
      </ShellSlot>
    </div>
  )
}
function SessionFacts({
  session,
  now,
}: {
  session: AgentSession
  now: number
}) {
  const [title, setTitle] = useState(session.title)
  const [editing, setEditing] = useState(false)
  const titleInput = useRef<HTMLInputElement>(null)
  useEffect(() => {
    if (editing) titleInput.current?.focus()
  }, [editing])
  const { report } = useStatus()
  const cache = useQueryClient()
  const rename = useMutation({
    mutationFn: () =>
      write(`/api/sessions/${session.id}`, "PATCH", { title }, nothing),
    onSuccess: async () => {
      setEditing(false)
      report("Session renamed")
      await cache.invalidateQueries({ queryKey: ["agent-sessions"] })
    },
    onError: (error) => report(message(error), "error"),
  })
  const span = (ms: number) =>
    ms >= 86400000
      ? `${Math.floor(ms / 86400000)}d ${Math.floor((ms % 86400000) / 3600000)}h`
      : ms >= 3600000
        ? `${Math.floor(ms / 3600000)}h ${Math.floor((ms % 3600000) / 60000)}m`
        : `${Math.max(0, Math.floor(ms / 60000))}m`
  return (
    <div className="session-summary">
      <div className="session-title">
        <h2>{session.title}</h2>
        <Button variant="ghost" size="xs" onClick={() => setEditing(!editing)}>
          Rename session
        </Button>
      </div>
      <p className="pane-meta">
        Started {date(session.startedAt)} · ran{" "}
        {span(Date.parse(session.lastSeenAt) - Date.parse(session.startedAt))} ·
        expires in {span(Date.parse(session.expiresAt) - now)}
      </p>
      {editing && (
        <form
          className="session-rename"
          onSubmit={(event) => {
            event.preventDefault()
            rename.mutate()
          }}
        >
          <Input
            aria-label="Session title"
            value={title}
            maxLength={160}
            ref={titleInput}
            onChange={(event) => setTitle(event.target.value)}
          />
          <Button
            type="submit"
            disabled={
              rename.isPending ||
              !title.trim() ||
              title.trim() === session.title
            }
          >
            Save title
          </Button>
          <Button
            variant="ghost"
            onClick={() => {
              setTitle(session.title)
              setEditing(false)
            }}
          >
            Cancel rename
          </Button>
        </form>
      )}
      <dl className="session-stats">
        {[
          { label: "executions", value: session.executions },
          { label: "tool calls", value: session.toolCalls },
          { label: "failed", value: session.failures },
          { label: "sources", value: session.sources.length },
        ].map((item) => (
          <div key={item.label}>
            <dt>{item.label}</dt>
            <dd
              className={
                item.label === "failed" && item.value ? "bad" : undefined
              }
            >
              {item.value}
            </dd>
          </div>
        ))}
      </dl>
    </div>
  )
}

const retentionMs = 96 * 60 * 60 * 1000
export function timelinePosition(time: string | number, now: number) {
  return Math.max(
    0,
    Math.min(
      100,
      ((new Date(time).getTime() - now + retentionMs) / retentionMs) * 100
    )
  )
}
function TimelineAxis({ now }: { now: number }) {
  return (
    <span className="timeline-axis">
      <span className="expires">← expires</span>
      {[1, 2, 3].map((day) => (
        <span key={day} style={{ left: `${day * 25}%` }}>
          {new Date(now - (4 - day) * 24 * 3600000).toLocaleDateString(
            undefined,
            { weekday: "short" }
          )}
        </span>
      ))}
      <span className="now">now</span>
    </span>
  )
}
function TimelineLane({
  session,
  ticks,
  now,
}: {
  session: AgentSession
  ticks: { id: string; startedAt: string; status: string }[]
  now: number
}) {
  const left = timelinePosition(session.startedAt, now)
  const right = timelinePosition(session.lastSeenAt, now)
  return (
    <figure className="timeline-track">
      <figcaption className="sr-only">{`${session.executions} executions, ${session.failures} failed; ${date(session.startedAt)} to ${date(session.lastSeenAt)}`}</figcaption>
      {[25, 50, 75].map((x) => (
        <i key={x} style={{ left: `${x}%` }} />
      ))}
      <span
        className={
          session.failures ? "timeline-bar has-failures" : "timeline-bar"
        }
        style={{ left: `${left}%`, width: `${right - left}%` }}
      />
      {ticks
        .filter(
          (tick) => new Date(tick.startedAt).getTime() >= now - retentionMs
        )
        .map((tick) => (
          <b
            key={tick.id}
            className={
              ["failed", "timed_out", "expired"].includes(tick.status)
                ? "failed"
                : undefined
            }
            style={{ left: `${timelinePosition(tick.startedAt, now)}%` }}
            title={`${date(tick.startedAt)} · ${tick.status}`}
          />
        ))}
    </figure>
  )
}
