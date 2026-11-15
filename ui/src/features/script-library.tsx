import { Breadcrumbs } from "@/components/breadcrumbs"
import { PythonCode } from "@/components/python-code"
import { useEffect, useRef } from "react"
import { Link, getRouteApi } from "@tanstack/react-router"
import {
  useQuery,
  useQueryClient,
  useSuspenseQuery,
} from "@tanstack/react-query"
import { Search } from "lucide-react"
import { workspacesQuery, scriptsQuery, scriptQuery } from "./queries"
import { DataList } from "@/components/data-list"
import { PaneStack } from "@/components/pane-stack"
import { TabBar } from "@/components/tab-bar"
import { ShellSlot } from "@/components/shell-slot"
import { Facts } from "@/components/facts"
import { ConfirmAction } from "@/components/confirm-action"
import { useStatus } from "@/components/status-bar"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { ErrorNotice, date } from "@/components/common"
import { nothing, write } from "@/lib/api"
import { useWorkspace } from "@/components/workspace-context"
const route = getRouteApi("/_authenticated/scripts")
export function ScriptLibraryPage() {
  const search = route.useSearch()
  const navigate = route.useNavigate()
  const workspaces = useSuspenseQuery(workspacesQuery).data
  const chosenWorkspace = useWorkspace()
  const workspace =
    workspaces.find((w) => w.id === (search.workspace ?? chosenWorkspace)) ??
    workspaces.find((w) => w.allSources) ??
    workspaces[0]
  const scripts = useQuery({
    ...scriptsQuery(workspace?.id ?? ""),
    enabled: !!workspace,
  })
  const rows = (scripts.data ?? []).filter((s) =>
    `${s.name} ${s.description}`
      .toLowerCase()
      .includes((search.q ?? "").toLowerCase())
  )
  const selected = rows.find((s) => s.id === search.selected) ?? rows[0]
  const detail = useQuery({
    ...scriptQuery(workspace?.id ?? "", selected?.id ?? ""),
    enabled: search.detail === "open" && !!workspace && !!selected,
  })
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
  const choose = (id: string, open = false) =>
    void navigate({
      search: {
        ...search,
        selected: id,
        detail: open || search.detail ? "open" : undefined,
      },
      replace: !open,
    })
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
      if (e.key === "/") {
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
        open={search.detail === "open" && !!selected}
        list={
          <>
            <ErrorNotice error={scripts.error} />
            <DataList
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
                    {s.name}
                  </span>,
                  s.description || "—",
                  date(s.updatedAt),
                ],
              }))}
              onSelect={choose}
              onOpen={(id) => choose(id, true)}
              onBack={back}
              empty={
                scripts.isPending
                  ? "Loading scripts…"
                  : search.q
                    ? "No matching scripts. Press Esc to clear."
                    : "No saved scripts. Save Python from a session or create a script."
              }
            />
          </>
        }
        detail={
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
                    <ConfirmAction
                      label="Delete script"
                      description="The script will no longer be available to agents. Its recorded executions remain."
                      onConfirm={async () => {
                        if (!detail.data) return
                        await write(
                          `/api/workspaces/${workspace.id}/scripts/${detail.data.id}`,
                          "DELETE",
                          { revision: detail.data.revision },
                          nothing
                        )
                        await cache.invalidateQueries({
                          queryKey: ["scripts", workspace.id],
                        })
                        back()
                      }}
                    />
                  </div>
                </>
              )}
              {detail.isPending && <p>Loading script…</p>}
            </div>
          </>
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
