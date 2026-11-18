import { move, navigationMotion } from "@/lib/motion"
import {
  useListCursor,
  useListSelection,
  listSearchKeys,
} from "@/hooks/use-list-navigation"
import { MatchText, SearchHint } from "@/components/match-text"
import { Breadcrumbs } from "@/components/breadcrumbs"
import { useEffect, useRef, useState } from "react"
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
} from "./queries"
import { ExecutionContent } from "./executions"
import { DataList, ListEmpty } from "@/components/data-list"
import { PaneStack } from "@/components/pane-stack"
import { TabBar } from "@/components/tab-bar"
import { ShellSlot } from "@/components/shell-slot"
import { useStatus } from "@/components/status-bar"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { relativeTime } from "@/lib/time"
import { ErrorNotice, date } from "@/components/common"
import { message, nothing, write } from "@/lib/api"
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
  const { report } = useStatus()
  const cache = useQueryClient()
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    const timer = setInterval(() => {
      if (document.visibilityState !== "hidden") setNow(Date.now())
    }, 5_000)
    return () => clearInterval(timer)
  }, [])
  const selected = cursor ?? rows[0]?.id
  const back = () => {
    move(openedExecution ? "pop" : "close", () => {
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
            q: undefined,
            runBefore: undefined,
          },
        })
        requestAnimationFrame(() => list.current?.focus())
      }
    })
  }
  const choose = (id: string, open = false) => {
    setCursor(id)
    if (!open) {
      if (opened) setOpened(id)
      return
    }
    move(opened ? "swap" : "open", () => {
      setOpened(id)
      void navigate({
        search: {
          ...search,
          selected: id,
          execution: undefined,
          runBefore: undefined,
        },
      })
    })
  }
  function openRun(id: string) {
    setRunCursor(id)
    move(openedExecution ? "swap" : "push", () => {
      setOpenedExecution(id)
      void navigate({ search: { ...search, selected: opened, execution: id } })
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
      } else if (event.key === "[" || event.key === "]") {
        event.preventDefault()
        void navigate({
          viewTransition: navigationMotion("filter"),
          search: {
            q: search.q,
            tab: search.tab === "errors" ? "all" : "errors",
          },
        })
      } else if (event.key === "Escape" || event.key === "ArrowLeft") {
        event.preventDefault()
        back()
      } else if (
        openedExecution &&
        ["ArrowDown", "ArrowUp", "j", "k"].includes(event.key)
      ) {
        const items = session.data?.executions ?? []
        const index = items.findIndex((e) => e.id === openedExecution)
        const next =
          items[
            Math.max(
              0,
              Math.min(
                items.length - 1,
                index +
                  (event.key === "ArrowDown" || event.key === "j" ? 1 : -1)
              )
            )
          ]
        if (next) {
          event.preventDefault()
          openRun(next.id)
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
            aria-describedby={
              /[*?]/.test(search.q ?? "") ? "list-search-hint" : undefined
            }
            aria-label="Search sessions"
            placeholder="Search sessions"
            value={search.q ?? ""}
            onChange={(e) =>
              void navigate({
                search: {
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
          {/[*?]/.test(search.q ?? "") && <SearchHint />}
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
            viewTransition: navigationMotion("filter"),
            search: {
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
              label="Sessions"
              listRef={list}
              columns={[
                "Session",
                "Agent",
                "Runs",
                "Calls",
                "Failures",
                "Last activity",
              ]}
              selectedId={selected}
              rows={rows.map((s) => ({
                id: s.id,
                label: s.title,
                group: new Date(s.lastSeenAt).toLocaleDateString(undefined, {
                  weekday: "long",
                  month: "short",
                  day: "numeric",
                }),
                cells: [
                  <MatchText key="title" query={search.q ?? ""}>
                    {s.title}
                  </MatchText>,
                  <MatchText key="client" query={search.q ?? ""}>
                    {s.client}
                  </MatchText>,
                  s.executions,
                  s.toolCalls,
                  <span key="status" className={s.failures ? "bad" : undefined}>
                    {s.failures ? `${s.failures} failed` : "—"}
                  </span>,
                  <time
                    key="lastSeen"
                    dateTime={s.lastSeenAt}
                    title={date(s.lastSeenAt)}
                  >
                    {relativeTime(s.lastSeenAt, now)}
                  </time>,
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
                  onOpen={openRun}
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
