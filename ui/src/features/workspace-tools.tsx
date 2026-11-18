import { move as transitionPane, navigationMotion } from "@/lib/motion"
import { BulkPane } from "@/components/bulk-pane"
import { ToolScopeMode } from "./policies"
import { MatchText, SearchHint } from "@/components/match-text"
import {
  useListCursor,
  useListSelection,
  listSearchKeys,
} from "@/hooks/use-list-navigation"
import { Breadcrumbs } from "@/components/breadcrumbs"
import { PythonCode } from "@/components/python-code"
import { useEffect, useMemo, useRef, useState } from "react"
import { Link, getRouteApi } from "@tanstack/react-router"
import {
  useMutation,
  useInfiniteQuery,
  useQueryClient,
  useSuspenseQuery,
} from "@tanstack/react-query"
import { Search, Layers } from "lucide-react"
import { workspacesQuery, sourcesQuery, policyInfiniteQuery } from "./queries"
import type { ToolPolicy } from "./queries"
import { DataList, ListEmpty } from "@/components/data-list"
import { PaneStack } from "@/components/pane-stack"
import { TabBar } from "@/components/tab-bar"
import { ShellSlot } from "@/components/shell-slot"
import { Facts } from "@/components/facts"
import { useStatus } from "@/components/status-bar"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { ErrorNotice } from "@/components/common"
import { nothing, write, message, bulkResult } from "@/lib/api"
import { useWorkspace } from "@/components/workspace-context"
const route = getRouteApi("/_authenticated/workspaces/")
export function WorkspaceToolsPage() {
  const search = route.useSearch()
  const navigate = route.useNavigate()
  const [cursor, setCursor] = useListCursor(
    search.selected,
    `${search.q ?? ""}:${search.workspace ?? ""}`
  )
  const [pane, setPane] = useListCursor(search.detail, search.selected)
  const workspaces = useSuspenseQuery(workspacesQuery).data
  const sources = useSuspenseQuery(sourcesQuery).data
  const chosenWorkspace = useWorkspace()
  const workspace =
    workspaces.find((w) => w.id === (search.workspace ?? chosenWorkspace)) ??
    workspaces.find((w) => w.allSources) ??
    workspaces[0]
  const selection = useListSelection(`${workspace?.id ?? ""}:${search.q ?? ""}`)
  const policies = useInfiniteQuery({
    ...policyInfiniteQuery(workspace?.id ?? "", search.q),
    enabled: !!workspace,
  })
  const tools = useMemo(
    () => policies.data?.pages.flatMap((page) => page.tools) ?? [],
    [policies.data]
  )
  const totalMatches = policies.data?.pages[0]?.totalMatches ?? 0
  const loadMore = policies.hasNextPage
    ? () => {
        if (!policies.isFetching && !policies.isFetchNextPageError)
          void policies.fetchNextPage({ cancelRefetch: false })
      }
    : undefined
  const selected = tools.find((t) => t.path === cursor) ?? tools[0]
  const list = useRef<HTMLTableElement>(null)
  const input = useRef<HTMLInputElement>(null)
  const cache = useQueryClient()
  const { report } = useStatus()
  const selectedTools = selection.value.allMatching
    ? tools
    : tools.filter((tool) => selection.value.ids.includes(tool.path))
  const selectedCount = selection.value.allMatching
    ? totalMatches
    : selection.value.ids.length
  const bulk = selectedCount > 1
  const applyBulk = async (action: string) => {
    if (!workspace) return
    const result = await write(
      `/api/workspaces/${workspace.id}/policies/bulk`,
      "POST",
      {
        selection: selection.value.allMatching
          ? { query: search.q ?? "" }
          : { ids: selection.value.ids },
        expectedCount: selectedCount,
        action,
      },
      bulkResult
    )
    await Promise.all([
      cache.invalidateQueries({ queryKey: ["policies"] }),
      cache.invalidateQueries({ queryKey: ["workspaces"] }),
    ])
    report(`${result.changed} tools updated`)
  }

  const back = () => {
    if (selection.value.allMatching || selection.value.ids.length > 1) {
      selection.set({
        ids: selected?.path ? [selected?.path] : [],
        allMatching: false,
      })
      return
    }
    transitionPane("close", () => {
      setPane("")
      void navigate({
        search: {
          ...search,
          detail: undefined,
          q: search.detail ? search.q : undefined,
        },
      })
      requestAnimationFrame(() => list.current?.focus())
    })
  }
  const choose = (path: string, open = false) => {
    setCursor(path)
    if (!open) return
    transitionPane(pane === "open" ? "swap" : "open", () => {
      setPane("open")
      void navigate({
        search: {
          ...search,
          selected: path,
          detail: open || search.detail ? "open" : undefined,
        },
        replace: !open,
      })
    })
  }
  const changeWorkspace = (id: string) =>
    void navigate({
      search: { workspace: id, q: search.q },
      viewTransition: navigationMotion("ws"),
    })
  const save = useMutation({
    mutationFn: ({
      tool,
      action,
      bulk,
    }: {
      tool: ToolPolicy
      action: "allow" | "block" | "default"
      bulk?: boolean
    }) => {
      if (!workspace) throw new Error("Choose a workspace first")
      return write(
        `/api/workspaces/${workspace.id}/policies${bulk ? "/source" : ""}`,
        "PUT",
        {
          sourceId: tool.sourceId,
          sourceRevision: tool.sourceRevision,
          ...(bulk ? {} : { upstreamName: tool.upstreamName }),
          action,
        },
        nothing
      )
    },
    onSuccess: async () => {
      report("Policy updated")
      await cache.invalidateQueries({ queryKey: ["policies"] })
    },
    onError: (e) => report(message(e), "error"),
  })
  const [scopeEdits, setScopeEdits] = useState<Record<string, boolean>>({})
  const clearScopeEdit = (workspaceId: string, path: string) =>
    setScopeEdits((current) => {
      const next = { ...current }
      delete next[`${workspaceId}:${path}`]
      return next
    })
  const scope = useMutation({
    mutationFn: ({
      tool,
      included,
      workspaceId,
    }: {
      tool: ToolPolicy
      included: boolean
      workspaceId: string
    }) =>
      write(
        `/api/workspaces/${workspaceId}/scope/tools`,
        "PUT",
        {
          sourceId: tool.sourceId,
          sourceRevision: tool.sourceRevision,
          upstreamName: tool.upstreamName,
          included,
        },
        nothing
      ),
    onSuccess: async (_, variables) => {
      await cache.invalidateQueries({
        queryKey: ["policies", variables.workspaceId],
      })
      clearScopeEdit(variables.workspaceId, variables.tool.path)
      report("Tool scope updated")
    },
    onError: (error, variables) => {
      clearScopeEdit(variables.workspaceId, variables.tool.path)
      report(message(error), "error")
    },
  })
  const toggle = () => {
    if (selected && selected.inScope && !save.isPending)
      save.mutate({
        tool: selected,
        action: selected.effectiveAction === "allow" ? "block" : "allow",
      })
  }
  useEffect(() => {
    const key = (event: KeyboardEvent) => {
      if (
        event.defaultPrevented ||
        event.ctrlKey ||
        event.metaKey ||
        event.altKey ||
        (event.target instanceof Element &&
          event.target.closest(
            "input,textarea,select,[contenteditable=true],#section-navigation"
          ))
      )
        return
      if (event.key === "/") {
        event.preventDefault()
        input.current?.focus()
      } else if (event.key === "Escape") {
        event.preventDefault()
        back()
      } else if (event.key === "[" || event.key === "]") {
        event.preventDefault()
        const at = workspaces.findIndex((w) => w.id === workspace?.id)
        changeWorkspace(
          workspaces[
            (at + (event.key === "]" ? 1 : workspaces.length - 1)) %
              workspaces.length
          ].id
        )
      } else if (search.detail && event.key === " ") {
        event.preventDefault()
        toggle()
      } else if (
        search.detail &&
        selected &&
        !save.isPending &&
        ["a", "b"].includes(event.key.toLowerCase())
      ) {
        event.preventDefault()
        save.mutate({
          tool: selected,
          action: event.key.toLowerCase() === "a" ? "allow" : "block",
          bulk: true,
        })
      }
    }
    document.addEventListener("keydown", key)
    return () => document.removeEventListener("keydown", key)
  })
  const { isPending: scopePending, mutate: saveScope } = scope
  const { isPending: policyPending, mutate: savePolicy } = save
  const rows = useMemo(
    () =>
      tools.map((t) => ({
        id: t.path,
        label: t.path,
        group: t.path.split(".")[0],
        cells: [
          <span key="path" className="tool-label font-mono">
            {!workspace?.allTools && (
              <input
                type="checkbox"
                aria-label={`Include ${t.path}`}
                checked={scopeEdits[`${workspace?.id}:${t.path}`] ?? t.inScope}
                disabled={scopePending}
                onClick={(event) => event.stopPropagation()}
                onKeyDown={(event) => event.stopPropagation()}
                onChange={(event) => {
                  const included = event.target.checked
                  setScopeEdits((current) => ({
                    ...current,
                    [`${workspace?.id}:${t.path}`]: included,
                  }))
                  saveScope({
                    tool: t,
                    included,
                    workspaceId: workspace?.id ?? "",
                  })
                }}
              />
            )}
            <MatchText query={search.q ?? ""}>{t.path}</MatchText>
          </span>,
          sources.find((s) => s.id === t.sourceId)?.slug ??
            t.path.split(".")[0],
          <button
            key="state"
            className={
              t.effectiveAction === "block"
                ? "policy-switch off"
                : "policy-switch"
            }
            role="switch"
            aria-checked={t.effectiveAction === "allow"}
            aria-label={`${t.effectiveAction === "allow" ? "Allowed" : "Blocked"}: ${t.path}`}
            disabled={policyPending || !t.inScope}
            onKeyDown={(event) => event.stopPropagation()}
            onClick={(event) => {
              event.stopPropagation()
              savePolicy({
                tool: t,
                action: t.effectiveAction === "allow" ? "block" : "allow",
              })
            }}
          >
            <i aria-hidden="true" />
            {!t.inScope
              ? "Outside scope"
              : t.effectiveAction === "allow"
                ? "Allowed"
                : "Blocked"}
          </button>,
        ],
      })),
    [
      tools,
      workspace,
      sources,
      scopeEdits,
      scopePending,
      saveScope,
      policyPending,
      savePolicy,
      search.q,
    ]
  )
  return (
    <div className="list-screen workspace-screen">
      <ShellSlot target="shell-search">
        <label className="global-search" htmlFor="workspace-search">
          <Search aria-hidden="true" />
          <Input
            id="workspace-search"
            ref={input}
            type="search"
            aria-describedby={
              /[*?]/.test(search.q ?? "") ? "list-search-hint" : undefined
            }
            aria-label="Search tools"
            placeholder="Search tools"
            value={search.q ?? ""}
            onChange={(e) =>
              void navigate({
                search: {
                  workspace: workspace?.id,
                  q: e.target.value || undefined,
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
            workspaceId={workspace?.id}
            items={[
              { label: "Workspaces", onSelect: back },
              ...(search.detail && selected ? [{ label: selected.path }] : []),
            ]}
          />
          <h1>{workspace?.name ?? "Workspaces"}</h1>
          <span className="list-total">{totalMatches} tools</span>
          <p>
            Choose which tools agents may call. Everything is allowed until you
            block it.
          </p>
        </div>
        <div className="detail-actions">
          {workspace && (
            <Button
              variant="outline"
              render={
                <Link
                  to="/workspaces/$workspaceId"
                  params={{ workspaceId: workspace.id }}
                />
              }
            >
              Manage {workspace.name}
            </Button>
          )}
          <Button render={<Link to="/workspaces/new" />}>New workspace</Button>
        </div>
      </header>
      {workspace && (
        <ToolScopeMode
          workspace={workspace}
          key={`${workspace.id}:${workspace.allTools}`}
        />
      )}
      <TabBar
        panelId="workspace-panes"
        label="Workspaces"
        selected={workspace?.id ?? ""}
        onSelect={changeWorkspace}
        tabs={workspaces.map((w) => ({
          id: w.id,
          label: w.name,
          count: sources
            .filter((s) => w.allSources || w.sourceIds.includes(s.id))
            .reduce((sum, s) => sum + s.toolCount, 0),
        }))}
      />
      <PaneStack
        id="workspace-panes"
        label="Workspace tool"
        open={bulk || (pane === "open" && !!selected)}
        list={
          <>
            <ErrorNotice error={policies.error} />
            <DataList
              selection={selection}
              totalMatches={totalMatches}
              onLoadMore={loadMore}
              label="Workspace tools"
              listRef={list}
              busy={policies.isFetching}
              columns={["Tool", "Source", "Policy"]}
              selectedId={selected?.path}
              rows={rows}
              onAction={(key) => {
                if (key !== "a" && key !== "b") return false
                if (bulk)
                  void applyBulk(key === "a" ? "allow" : "block").catch(
                    (error) => report(message(error), "error")
                  )
                else if (selected)
                  save.mutate({
                    tool: selected,
                    action: key === "a" ? "allow" : "block",
                  })
                return true
              }}
              onSelect={choose}
              onOpen={(id) => choose(id, true)}
              onBack={back}
              empty={
                policies.isPending ? (
                  "Loading tools…"
                ) : search.q ? (
                  <ListEmpty
                    title="No matching tools"
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
                    icon={<Layers strokeWidth={1} aria-hidden="true" />}
                    title="No tools in this workspace"
                    description="Connect an integration or add existing sources to this workspace."
                  >
                    <Button render={<Link to="/integrations" />}>
                      Browse integrations
                    </Button>
                    {workspace && (
                      <Button
                        variant="outline"
                        render={
                          <Link
                            to="/workspaces/$workspaceId"
                            params={{ workspaceId: workspace.id }}
                          />
                        }
                      >
                        Manage workspace
                      </Button>
                    )}
                  </ListEmpty>
                )
              }
            />
            {policies.isFetchingNextPage && (
              <output className="list-loading">Loading more tools…</output>
            )}
            {policies.isFetchNextPageError && (
              <Button
                variant="outline"
                onClick={() =>
                  void policies.fetchNextPage({ cancelRefetch: false })
                }
              >
                Retry loading tools
              </Button>
            )}
          </>
        }
        detail={
          bulk ? (
            <BulkPane
              key={JSON.stringify([selection.value, selectedCount])}
              count={selectedCount}
              names={selectedTools.map((tool) => tool.path)}
              allMatching={selection.value.allMatching}
              actions={[
                { id: "allow", label: "Allow" },
                { id: "block", label: "Block" },
                {
                  id: "include",
                  label: "Include",
                  disabled: workspace?.allTools,
                },
                {
                  id: "exclude",
                  label: "Exclude",
                  disabled: workspace?.allTools,
                },
              ]}
              onAction={applyBulk}
              onClose={() =>
                selection.set({
                  ids: selected ? [selected.path] : [],
                  allMatching: false,
                })
              }
            />
          ) : (
            selected &&
            workspace && (
              <>
                <div className="pane-heading">
                  <span>Tool policy</span>
                  <Button variant="ghost" onClick={back}>
                    Close <span className="kbd">Esc</span>
                  </Button>
                </div>
                <div className="pane-body">
                  <h2 className="font-mono">{selected.path}</h2>
                  <p className="pane-meta">
                    {selected.inScope
                      ? selected.effectiveAction === "allow"
                        ? "Allowed"
                        : "Blocked"
                      : "Outside scope"}{" "}
                    in {workspace.name}
                  </p>
                  <h3>Signature</h3>
                  <pre className="code-panel">
                    <PythonCode code={selected.signature} />
                  </pre>
                  <p>{selected.description}</p>
                  <Facts
                    items={[
                      {
                        label: "Read only",
                        value: selected.readOnly ? "Yes" : "No",
                      },
                      {
                        label: "Destructive",
                        value: selected.destructive ? "Yes" : "No",
                      },
                    ]}
                  />
                  <div className="detail-actions">
                    <Button
                      variant="ghost"
                      disabled={save.isPending || !selected.overrideAction}
                      onClick={() =>
                        save.mutate({ tool: selected, action: "default" })
                      }
                    >
                      Use default
                    </Button>
                    <Button
                      disabled={save.isPending || !selected.inScope}
                      onClick={toggle}
                    >
                      {selected.effectiveAction === "allow" ? "Block" : "Allow"}
                      <span className="kbd">Space</span>
                    </Button>
                    <Button
                      variant="ghost"
                      disabled={save.isPending}
                      onClick={() =>
                        save.mutate({
                          tool: selected,
                          action: "allow",
                          bulk: true,
                        })
                      }
                    >
                      Allow all from source <span className="kbd">A</span>
                    </Button>
                    <Button
                      variant="ghost"
                      disabled={save.isPending}
                      onClick={() =>
                        save.mutate({
                          tool: selected,
                          action: "block",
                          bulk: true,
                        })
                      }
                    >
                      Block all from source <span className="kbd">B</span>
                    </Button>
                  </div>
                  <p>
                    A blocked call raises <code>PermissionError</code> in the
                    agent’s Python, naming the tool and workspace. Source and
                    tool scopes still apply.
                  </p>
                </div>
              </>
            )
          )
        }
      />
      <ShellSlot target="status-keys">
        <button
          disabled={!selected}
          onClick={() => selected && choose(selected.path, true)}
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
