import { Breadcrumbs } from "@/components/breadcrumbs"
import { PythonCode } from "@/components/python-code"
import { useEffect, useRef } from "react"
import { Link, getRouteApi } from "@tanstack/react-router"
import {
  useMutation,
  useQuery,
  useQueryClient,
  useSuspenseQuery,
} from "@tanstack/react-query"
import { Search } from "lucide-react"
import { workspacesQuery, sourcesQuery, policyQuery } from "./queries"
import type { ToolPolicy } from "./queries"
import { DataList } from "@/components/data-list"
import { PaneStack } from "@/components/pane-stack"
import { TabBar } from "@/components/tab-bar"
import { ShellSlot } from "@/components/shell-slot"
import { Facts } from "@/components/facts"
import { useStatus } from "@/components/status-bar"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { ErrorNotice } from "@/components/common"
import { nothing, write, message } from "@/lib/api"
import { useWorkspace } from "@/components/workspace-context"
const route = getRouteApi("/_authenticated/workspaces/")
export function WorkspaceToolsPage() {
  const search = route.useSearch()
  const navigate = route.useNavigate()
  const workspaces = useSuspenseQuery(workspacesQuery).data
  const sources = useSuspenseQuery(sourcesQuery).data
  const chosenWorkspace = useWorkspace()
  const workspace =
    workspaces.find((w) => w.id === (search.workspace ?? chosenWorkspace)) ??
    workspaces.find((w) => w.allSources) ??
    workspaces[0]
  const policies = useQuery({
    ...policyQuery(workspace?.id ?? "", search.q, search.after),
    enabled: !!workspace,
  })
  const tools = policies.data?.tools ?? []
  const selected = tools.find((t) => t.path === search.selected) ?? tools[0]
  const list = useRef<HTMLTableElement>(null)
  const input = useRef<HTMLInputElement>(null)
  const cache = useQueryClient()
  const { report } = useStatus()
  const back = () => {
    void navigate({
      search: {
        ...search,
        detail: undefined,
        q: search.detail ? search.q : undefined,
      },
    })
    requestAnimationFrame(() => list.current?.focus())
  }
  const choose = (path: string, open = false) =>
    void navigate({
      search: {
        ...search,
        selected: path,
        detail: open || search.detail ? "open" : undefined,
      },
      replace: !open,
    })
  const changeWorkspace = (id: string) =>
    void navigate({ search: { workspace: id, q: search.q } })
  const save = useMutation({
    mutationFn: ({
      tool,
      action,
      bulk,
    }: {
      tool: ToolPolicy
      action: "allow" | "block"
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
  return (
    <div className="list-screen workspace-screen">
      <ShellSlot target="shell-search">
        <label className="global-search" htmlFor="workspace-search">
          <Search aria-hidden="true" />
          <Input
            id="workspace-search"
            ref={input}
            type="search"
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
            workspaceId={workspace?.id}
            items={[
              { label: "Workspaces", onSelect: back },
              ...(search.detail && selected ? [{ label: selected.path }] : []),
            ]}
          />
          <h1>Workspaces</h1>
          <p>
            Choose which tools agents may call. Everything is allowed until you
            block it.
          </p>
        </div>
        <Button render={<Link to="/workspaces/new" />}>New workspace</Button>
      </header>
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
        open={search.detail === "open" && !!selected}
        list={
          <>
            <ErrorNotice error={policies.error} />
            <DataList
              label="Workspace tools"
              listRef={list}
              busy={policies.isFetching}
              columns={["Tool", "Source", "Policy"]}
              selectedId={selected?.path}
              rows={tools.map((t) => ({
                id: t.path,
                label: t.path,
                group: t.path.split(".")[0],
                cells: [
                  <span key="path" className="font-mono">
                    {t.path}
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
                    disabled={save.isPending || !t.inScope}
                    onKeyDown={(event) => event.stopPropagation()}
                    onClick={(event) => {
                      event.stopPropagation()
                      save.mutate({
                        tool: t,
                        action:
                          t.effectiveAction === "allow" ? "block" : "allow",
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
              }))}
              onSelect={choose}
              onOpen={(id) => choose(id, true)}
              onBack={back}
              empty={
                policies.isPending
                  ? "Loading tools…"
                  : "No matching tools. Connect a source or change this workspace’s source selection."
              }
            />
            <div className="pane-pagination">
              <Button
                variant="ghost"
                disabled={!search.after}
                onClick={() =>
                  void navigate({
                    search: { workspace: workspace?.id, q: search.q },
                  })
                }
              >
                First page
              </Button>
              <Button
                variant="ghost"
                disabled={!policies.data?.nextCursor}
                onClick={() =>
                  void navigate({
                    search: {
                      workspace: workspace?.id,
                      q: search.q,
                      after: policies.data?.nextCursor ?? undefined,
                    },
                  })
                }
              >
                Next page
              </Button>
              {workspace && (
                <Button
                  variant="ghost"
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
            </div>
          </>
        }
        detail={
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
                  agent’s Python, naming the tool and workspace. Source and tool
                  scopes still apply.
                </p>
              </div>
            </>
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
