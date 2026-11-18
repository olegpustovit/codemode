import { flushSync } from "react-dom"
import { BulkPane } from "@/components/bulk-pane"
import { matches } from "@/lib/match"
import { MatchText, SearchHint } from "@/components/match-text"
import {
  useListCursor,
  useListSelection,
  listSearchKeys,
} from "@/hooks/use-list-navigation"
import { Breadcrumbs } from "@/components/breadcrumbs"
import { PythonCode } from "@/components/python-code"
import { useEffect, useRef, useState } from "react"
import { Link, getRouteApi } from "@tanstack/react-router"
import {
  useQuery,
  useQueryClient,
  useSuspenseQuery,
} from "@tanstack/react-query"
import { Search, FileCode2 } from "lucide-react"
import {
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
  EmptyDescription,
  EmptyContent,
} from "@/components/ui/empty"
import {
  workspacesQuery,
  scriptsQuery,
  scriptQuery,
  scriptSchema,
} from "./queries"
import { DataList } from "@/components/data-list"
import { PaneStack } from "@/components/pane-stack"
import { TabBar } from "@/components/tab-bar"
import { ShellSlot } from "@/components/shell-slot"
import { Facts } from "@/components/facts"
import { useStatus } from "@/components/status-bar"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { ErrorNotice, date } from "@/components/common"
import { write, bulkResult } from "@/lib/api"
import { useWorkspace } from "@/components/workspace-context"
const route = getRouteApi("/_authenticated/scripts")
export function ScriptLibraryPage() {
  const search = route.useSearch()
  const navigate = route.useNavigate()
  const [cursor, setCursor] = useListCursor(
    search.selected,
    `${search.q ?? ""}:${search.workspace ?? ""}`
  )
  const [pane, setPane] = useListCursor(search.detail, search.selected)
  const workspaces = useSuspenseQuery(workspacesQuery).data
  const chosenWorkspace = useWorkspace()
  const workspace =
    workspaces.find((w) => w.id === (search.workspace ?? chosenWorkspace)) ??
    workspaces.find((w) => w.allSources) ??
    workspaces[0]
  const selection = useListSelection(`${workspace?.id ?? ""}:${search.q ?? ""}`)
  const scripts = useQuery({
    ...scriptsQuery(workspace?.id ?? ""),
    enabled: !!workspace,
  })
  const rows = (scripts.data ?? []).filter((s) =>
    matches(search.q ?? "", s.name, s.description)
  )
  const selected = rows.find((s) => s.id === cursor) ?? rows[0]
  const detail = useQuery({
    ...scriptQuery(workspace?.id ?? "", selected?.id ?? ""),
    enabled: pane === "open" && !!workspace && !!selected,
  })
  const list = useRef<HTMLTableElement>(null)
  const input = useRef<HTMLInputElement>(null)
  const cache = useQueryClient()
  const { report } = useStatus()
  const [deleting, setDeleting] = useState(false)
  const selectedScripts = selection.value.allMatching
    ? rows
    : rows.filter((script) => selection.value.ids.includes(script.id))
  const selectedCount = selectedScripts.length || (selected ? 1 : 0)
  const bulk = selectedScripts.length > 1 || deleting
  const deleteSelection = selectedScripts.length
    ? selectedScripts
    : selected
      ? [selected]
      : []

  const back = () => {
    if (selection.value.allMatching || selection.value.ids.length > 1) {
      selection.set({
        ids: selected?.id ? [selected?.id] : [],
        allMatching: false,
      })
      return
    }
    setDeleting(false)
    setPane("")
    void navigate({
      search: {
        ...search,
        detail: undefined,
        q: search.detail ? search.q : undefined,
      },
    })
    requestAnimationFrame(() => list.current?.focus())
  }
  const choose = (id: string, open = false) => {
    setCursor(id)
    if (!open) return
    flushSync(() => setPane("open"))
    void navigate({
      search: {
        ...search,
        selected: id,
        detail: open || search.detail ? "open" : undefined,
      },
      replace: !open,
    })
  }
  useEffect(() => {
    const keys = (e: KeyboardEvent) => {
      if (
        e.defaultPrevented ||
        e.ctrlKey ||
        e.altKey ||
        e.metaKey ||
        (e.target instanceof Element &&
          e.target.closest(
            "input,textarea,select,[contenteditable=true],#section-navigation"
          ))
      )
        return
      if (e.key.toLowerCase() === "n" && workspace) {
        e.preventDefault()
        void navigate({
          to: "/workspaces/$workspaceId/scripts/new",
          params: { workspaceId: workspace.id },
        })
      } else if (e.key === "/") {
        e.preventDefault()
        input.current?.focus()
      } else if (e.key === "Escape") {
        e.preventDefault()
        back()
      }
    }
    document.addEventListener("keydown", keys)
    return () => document.removeEventListener("keydown", keys)
  })
  return (
    <div className="list-screen scripts-screen">
      <ShellSlot target="shell-search">
        <label className="global-search" htmlFor="scripts-search">
          <Search aria-hidden="true" />
          <Input
            id="scripts-search"
            ref={input}
            type="search"
            aria-describedby={
              /[*?]/.test(search.q ?? "") ? "list-search-hint" : undefined
            }
            aria-label="Search scripts"
            placeholder="Search scripts"
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
              { label: "Scripts", onSelect: back },
              ...(search.detail && selected ? [{ label: selected.name }] : []),
            ]}
          />
          <h1>Scripts</h1>
          <p>
            Python saved from sessions or written by hand. Agents can run them
            by name.
          </p>
        </div>
        {workspace && (
          <Button
            render={
              <Link
                to="/workspaces/$workspaceId/scripts/new"
                params={{ workspaceId: workspace.id }}
              />
            }
          >
            New script
          </Button>
        )}
      </header>
      <TabBar
        label="Filter scripts"
        panelId="script-panes"
        selected="all"
        onSelect={() => {}}
        tabs={[{ id: "all", label: "All", count: scripts.data?.length ?? 0 }]}
      />
      <PaneStack
        id="script-panes"
        label="Script"
        open={bulk || (pane === "open" && !!selected)}
        list={
          <>
            <ErrorNotice error={scripts.error} />
            <DataList
              selection={selection}
              label="Scripts"
              listRef={list}
              busy={scripts.isFetching}
              selectedId={selected?.id}
              columns={["Script", "Description", "Saved"]}
              rows={rows.map((s) => ({
                id: s.id,
                label: s.name,
                group: "Saved scripts",
                cells: [
                  <span key="name" className="font-mono">
                    <MatchText query={search.q ?? ""}>{s.name}</MatchText>
                  </span>,
                  s.description || "—",
                  date(s.updatedAt),
                ],
              }))}
              onDelete={() => setDeleting(true)}
              onRename={async (id, name) => {
                if (!workspace) return
                const script = await cache.fetchQuery(
                  scriptQuery(workspace.id, id)
                )
                await write(
                  `/api/workspaces/${workspace.id}/scripts/${id}`,
                  "PUT",
                  {
                    name,
                    description: script.description,
                    code: script.code,
                    revision: script.revision,
                  },
                  scriptSchema
                )
                await cache.invalidateQueries({
                  queryKey: ["scripts", workspace.id],
                })
                report("Script renamed")
              }}
              onSelect={choose}
              onOpen={(id) => choose(id, true)}
              onBack={back}
              empty={
                scripts.isPending ? (
                  "Loading scripts…"
                ) : search.q ? (
                  "No matching scripts. Press Esc to clear."
                ) : (
                  <>
                    <EmptyHeader>
                      <EmptyMedia>
                        <FileCode2 strokeWidth={1} aria-hidden="true" />
                      </EmptyMedia>
                      <EmptyTitle>No saved scripts</EmptyTitle>
                      <EmptyDescription>
                        Keep Python you want to run again. Write a script or
                        save code from a session.
                      </EmptyDescription>
                    </EmptyHeader>
                    <EmptyContent className="empty-actions">
                      {workspace && (
                        <Button
                          render={
                            <Link
                              to="/workspaces/$workspaceId/scripts/new"
                              params={{ workspaceId: workspace.id }}
                            />
                          }
                        >
                          New script
                        </Button>
                      )}
                      <Button
                        variant="outline"
                        render={<Link to="/sessions" />}
                      >
                        Open Sessions
                      </Button>
                    </EmptyContent>
                  </>
                )
              }
            />
          </>
        }
        detail={
          bulk ? (
            <BulkPane
              key={JSON.stringify([selection.value, deleting])}
              count={selectedCount}
              names={deleteSelection.map((script) => script.name)}
              allMatching={selection.value.allMatching}
              confirmAction={deleting ? "delete" : undefined}
              actions={[
                {
                  id: "delete",
                  label: "Delete scripts",
                  confirm:
                    "The selected scripts will no longer be available to agents. Recorded executions remain.",
                },
              ]}
              onClose={() => {
                setDeleting(false)
                selection.set({
                  ids: selected ? [selected.id] : [],
                  allMatching: false,
                })
              }}
              onAction={async () => {
                if (!workspace) return
                const result = await write(
                  `/api/workspaces/${workspace.id}/scripts/bulk-delete`,
                  "POST",
                  {
                    selection: selection.value.allMatching
                      ? { query: search.q ?? "" }
                      : { ids: deleteSelection.map((script) => script.id) },
                    expectedCount: selectedCount,
                    revisions: deleteSelection.map((script) => ({
                      id: script.id,
                      revision: script.revision,
                    })),
                  },
                  bulkResult
                )
                selection.set({ ids: [], allMatching: false })
                setDeleting(false)
                setPane("")
                void navigate({
                  search: { ...search, selected: undefined, detail: undefined },
                  replace: true,
                })
                requestAnimationFrame(() => list.current?.focus())
                await cache.invalidateQueries({
                  queryKey: ["scripts", workspace.id],
                })
                report(`${result.changed} scripts deleted`)
              }}
            />
          ) : (
            <>
              <div className="pane-heading">
                <span>Script</span>
                <Button variant="ghost" onClick={back}>
                  Close <span className="kbd">Esc</span>
                </Button>
              </div>
              <div className="pane-body">
                <ErrorNotice error={detail.error} />
                {detail.data && workspace && (
                  <>
                    <h2 className="font-mono">{detail.data.name}</h2>
                    <p className="pane-meta">{detail.data.description}</p>
                    <Facts
                      items={[
                        { label: "Workspace", value: workspace.name },
                        { label: "Saved", value: date(detail.data.updatedAt) },
                        { label: "Revision", value: detail.data.revision },
                      ]}
                    />
                    <h3>Code</h3>
                    <pre className="code-panel">
                      <PythonCode code={detail.data.code} />
                    </pre>
                    <div className="detail-actions">
                      <Button
                        render={
                          <Link
                            to="/workspaces/$workspaceId/scripts/$scriptId"
                            params={{
                              workspaceId: workspace.id,
                              scriptId: detail.data.id,
                            }}
                          />
                        }
                      >
                        Edit
                      </Button>
                      <Button
                        variant="ghost"
                        onClick={() =>
                          void navigator.clipboard
                            .writeText(detail.data.code)
                            .then(() => report("Code copied"))
                            .catch(() =>
                              report(
                                "Copy failed. Select the code and copy it.",
                                "error"
                              )
                            )
                        }
                      >
                        Copy
                      </Button>
                      <Button
                        variant="destructive"
                        onClick={() => setDeleting(true)}
                      >
                        Delete script
                      </Button>
                    </div>
                  </>
                )}
                {detail.isPending && <p>Loading script…</p>}
              </div>
            </>
          )
        }
      />
      <ShellSlot target="status-keys">
        <button
          disabled={!selected}
          onClick={() => selected && choose(selected.id, true)}
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
