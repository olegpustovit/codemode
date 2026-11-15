import { flushSync } from "react-dom"
import {
  useListCursor,
  useListSelection,
  listSearchKeys,
} from "@/hooks/use-list-navigation"
import { MatchText, SearchHint } from "@/components/match-text"
import { Breadcrumbs } from "@/components/breadcrumbs"
import { useEffect, useMemo, useRef, useState } from "react"
import { Link, getRouteApi } from "@tanstack/react-router"
import {
  useInfiniteQuery,
  useMutation,
  useQuery,
  useQueryClient,
  useSuspenseInfiniteQuery,
} from "@tanstack/react-query"
import { Search, Activity } from "lucide-react"
import type { AgentSession } from "./queries"
import {
  agentSessionsInfiniteQuery,
  agentSessionInfiniteQuery,
  executionQuery,
  sessionActivitySchema,
} from "./queries"
import { ExecutionContent } from "./executions"
import { DataList, ListEmpty } from "@/components/data-list"
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
  const pages = useSuspenseInfiniteQuery(
    agentSessionsInfiniteQuery(search.q, search.tab === "errors")
  )
  const rows = pages.data.pages.flatMap((page) => page.sessions)
  const totalMatches = pages.data.pages[0]?.totalMatches ?? 0
  const loadMore = pages.hasNextPage
    ? () => {
        if (!pages.isFetching && !pages.isFetchNextPageError)
          void pages.fetchNextPage({ cancelRefetch: false })
      }
    : undefined
  const [cursor, setCursor] = useListCursor(
    search.selected ?? search.cursor,
    `${search.q ?? ""}:${search.tab ?? ""}`
  )
  const [opened, setOpened] = useListCursor(search.selected)
  const [runCursor, setRunCursor] = useListCursor(
    search.execution ?? search.runCursor,
    opened
  )
  const [openedExecution, setOpenedExecution] = useListCursor(
    search.execution,
    opened
  )
  const selection = useListSelection(`${search.q ?? ""}:${search.tab ?? ""}`)
  const input = useRef<HTMLInputElement>(null)
  const list = useRef<HTMLTableElement>(null)
  const runs = useRef<HTMLTableElement>(null)
  const session = useInfiniteQuery({
    ...agentSessionInfiniteQuery(opened ?? ""),
    enabled: !!opened,
    select: (data) => ({
      ...data.pages[0],
      executions: data.pages.flatMap((page) => page.executions),
    }),
  })
  const execution = useQuery({
    ...executionQuery(openedExecution ?? ""),
    enabled:
      !!openedExecution &&
      session.data?.executions.some((e) => e.id === openedExecution) === true,
  })
  const {
    data: openedSession,
    hasNextPage: moreExecutions,
    isFetching: loadingExecutions,
    isFetchNextPageError: executionPageError,
    fetchNextPage: fetchExecutions,
  } = session
  const {
    hasNextPage: moreSessions,
    isFetching: loadingSessions,
    isFetchNextPageError: sessionPageError,
    fetchNextPage: fetchSessions,
  } = pages
  useEffect(() => {
    if (
      openedExecution &&
      openedSession &&
      !openedSession.executions.some(
        (execution) => execution.id === openedExecution
      ) &&
      moreExecutions &&
      !loadingExecutions &&
      !executionPageError
    )
      void fetchExecutions({ cancelRefetch: false })
  }, [
    openedExecution,
    openedSession,
    moreExecutions,
    loadingExecutions,
    executionPageError,
    fetchExecutions,
  ])
  useEffect(() => {
    if (
      opened &&
      !rows.some((row) => row.id === opened) &&
      moreSessions &&
      !loadingSessions &&
      !sessionPageError
    )
      void fetchSessions({ cancelRefetch: false })
  }, [
    opened,
    rows,
    moreSessions,
    loadingSessions,
    sessionPageError,
    fetchSessions,
  ])
  const timeline = search.view === "timeline" && !openedExecution
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
  const { report } = useStatus()
  const cache = useQueryClient()
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
  const selected = cursor ?? rows[0]?.id
  const back = () => {
    if (openedExecution) {
      setOpenedExecution("")
      void navigate({ search: { ...search, execution: undefined } })
      requestAnimationFrame(() => runs.current?.focus())
    } else {
      setOpened("")
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
  const choose = (id: string, open = false) => {
    setCursor(id)
    if (open) flushSync(() => setOpened(id))
    else if (opened) setOpened(id)
    if (!open) return
    void navigate({
      search: {
        ...search,
        selected: id,
        execution: undefined,
        runBefore: undefined,
      },
      replace: !open,
    })
  }
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
        openedExecution &&
        (event.key === "ArrowDown" || event.key === "ArrowUp")
      ) {
        const items = session.data?.executions ?? []
        const index = items.findIndex((e) => e.id === openedExecution)
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
          setRunCursor(next.id)
          setOpenedExecution(next.id)
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
            aria-describedby="list-search-hint"
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
            onKeyDown={(event) =>
              listSearchKeys(
                event,
                list.current,
                () =>
                  void navigate({
                    search: { ...search, q: undefined },
                    replace: true,
                  })
              )
            }
          />
          <SearchHint />
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
                      onSelect: openedExecution
                        ? () =>
                            void navigate({
                              search: { ...search, execution: undefined },
                            })
                        : undefined,
                    },
                  ]
                : []),
              ...(openedExecution
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
          {
            id: "all",
            label: "All",
            count: pages.data.pages[0]?.allMatches ?? 0,
          },
          {
            id: "errors",
            label: "With errors",
            count: pages.data.pages[0]?.errorMatches ?? 0,
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
        open={!!opened}
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
                      <MatchText key="title" query={search.q ?? ""}>
                        {s.title}
                      </MatchText>,
                      <TimelineLane
                        key="track"
                        session={s}
                        ticks={ticks.get(s.id) ?? []}
                        now={now}
                      />,
                    ]
                  : [
                      <MatchText key="title" query={search.q ?? ""}>
                        {s.title}
                      </MatchText>,
                      <MatchText key="client" query={search.q ?? ""}>
                        {s.client}
                      </MatchText>,
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
              selection={selection}
              totalMatches={totalMatches}
              onLoadMore={loadMore}
              onRename={async (id, title) => {
                await write(`/api/sessions/${id}`, "PATCH", { title }, nothing)
                await cache.invalidateQueries({ queryKey: ["agent-sessions"] })
                report("Session renamed")
              }}
              onSelect={(id) => choose(id)}
              onOpen={(id) => choose(id, true)}
              onBack={back}
              empty={
                search.q ? (
                  <ListEmpty
                    title="No matching sessions"
                    description="Try another name or pattern. Esc clears your search."
                  >
                    <Button
                      variant="outline"
                      onClick={() =>
                        void navigate({
                          search: { ...search, q: undefined },
                          replace: true,
                        })
                      }
                    >
                      Clear search
                    </Button>
                  </ListEmpty>
                ) : (
                  <ListEmpty
                    icon={<Activity strokeWidth={1} aria-hidden="true" />}
                    title={
                      search.tab === "errors"
                        ? "No sessions with errors"
                        : "No sessions yet"
                    }
                    description={
                      search.tab === "errors"
                        ? "Sessions with failed executions appear here."
                        : "Connect an agent and run Python. Its activity will appear here."
                    }
                  >
                    <Button
                      render={<Link to="/tokens" search={{ detail: "new" }} />}
                    >
                      Create personal token
                    </Button>
                    <Button
                      variant="outline"
                      render={<Link to="/integrations" />}
                    >
                      Browse integrations
                    </Button>
                  </ListEmpty>
                )
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
            <ErrorNotice error={pages.error} />
            {pages.isFetchingNextPage && (
              <output className="list-loading">Loading more sessions…</output>
            )}
            {pages.isFetchNextPageError && (
              <Button
                variant="outline"
                onClick={() =>
                  void pages.fetchNextPage({ cancelRefetch: false })
                }
              >
                Retry loading sessions
              </Button>
            )}
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
                  selectedId={runCursor}
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
                  onSelect={(id) => {
                    setRunCursor(id)
                    if (openedExecution) setOpenedExecution(id)
                  }}
                  onOpen={(id) => {
                    setRunCursor(id)
                    setOpenedExecution(id)
                    void navigate({
                      search: { ...search, selected: opened, execution: id },
                    })
                  }}
                  onLoadMore={
                    session.hasNextPage
                      ? () => {
                          if (
                            !session.isFetching &&
                            !session.isFetchNextPageError
                          )
                            void session.fetchNextPage({ cancelRefetch: false })
                        }
                      : undefined
                  }
                  onBack={back}
                />
                {session.isFetchingNextPage && (
                  <output className="list-loading">
                    Loading more executions…
                  </output>
                )}
                {session.isFetchNextPageError && (
                  <Button
                    variant="outline"
                    onClick={() =>
                      void session.fetchNextPage({ cancelRefetch: false })
                    }
                  >
                    Retry loading executions
                  </Button>
                )}
              </>
            )}
          </>
        }
        execution={
          openedExecution ? (
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
