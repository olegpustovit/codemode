import { Breadcrumbs } from "@/components/breadcrumbs"
import { useEffect, useRef } from "react"
import { Link, getRouteApi } from "@tanstack/react-router"
import { useQueryClient, useSuspenseQuery } from "@tanstack/react-query"
import { Search } from "lucide-react"
import { tokensQuery, workspacesQuery } from "./queries"
import { TokenForm } from "./tokens"
import { DataList } from "@/components/data-list"
import { PaneStack } from "@/components/pane-stack"
import { TabBar } from "@/components/tab-bar"
import { ShellSlot } from "@/components/shell-slot"
import { Facts } from "@/components/facts"
import { ConfirmAction } from "@/components/confirm-action"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { date } from "@/components/common"
import { nothing, write } from "@/lib/api"
const route = getRouteApi("/_authenticated/tokens")
export function TokensPage() {
  const search = route.useSearch()
  const navigate = route.useNavigate()
  const tokens = useSuspenseQuery(tokensQuery).data
  const workspaces = useSuspenseQuery(workspacesQuery).data
  const rows = tokens.filter((t) =>
    t.name.toLowerCase().includes((search.q ?? "").toLowerCase())
  )
  const selected = rows.find((t) => t.id === search.selected) ?? rows[0]
  const list = useRef<HTMLTableElement>(null)
  const input = useRef<HTMLInputElement>(null)
  const cache = useQueryClient()
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
        detail: open || search.detail === "open" ? "open" : undefined,
      },
      replace: !open,
    })
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (
        e.defaultPrevented ||
        e.ctrlKey ||
        e.metaKey ||
        e.altKey ||
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
    document.addEventListener("keydown", key)
    return () => document.removeEventListener("keydown", key)
  })
  return (
    <div className="list-screen tokens-screen">
      <ShellSlot target="shell-search">
        <label className="global-search" htmlFor="tokens-search">
          <Search aria-hidden="true" />
          <Input
            id="tokens-search"
            ref={input}
            type="search"
            aria-label="Search agents"
            placeholder="Search agents"
            value={search.q ?? ""}
            onChange={(e) =>
              void navigate({
                search: { q: e.target.value || undefined },
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
            items={[
              { label: "Tokens", onSelect: back },
              ...(search.detail === "new"
                ? [{ label: "New personal token" }]
                : search.detail && selected
                  ? [{ label: selected.name }]
                  : []),
            ]}
          />
          <h1>Tokens</h1>
          <p>
            Agents holding access. Revoking takes effect on their next request.
          </p>
        </div>
        <Button
          onClick={() =>
            void navigate({ search: { ...search, detail: "new" } })
          }
        >
          New personal token
        </Button>
      </header>
      <TabBar
        label="Filter tokens"
        panelId="token-panes"
        selected="all"
        onSelect={() => {}}
        tabs={[{ id: "all", label: "All", count: tokens.length }]}
      />
      <PaneStack
        id="token-panes"
        label="Token"
        open={
          search.detail === "new" || (search.detail === "open" && !!selected)
        }
        list={
          <DataList
            label="Agents with access"
            listRef={list}
            selectedId={selected?.id}
            columns={["Agent", "Type", "Workspace", "Last used"]}
            rows={rows
              .map((t) => ({
                id: t.id,
                label: t.name,
                group: t.revoked ? "Revoked access" : "Agents with access",
                cells: [
                  t.name,
                  t.oauth ? "OAuth" : "Personal token",
                  t.resource === "/mcp-admin"
                    ? "Administration"
                    : (workspaces.find((w) => w.id === t.workspaceId)?.name ??
                      "Deleted workspace"),
                  t.lastUsedAt ? date(t.lastUsedAt) : "Never",
                ],
              }))
              .sort((a, b) => a.group.localeCompare(b.group))}
            onSelect={choose}
            onOpen={(id) => choose(id, true)}
            onBack={back}
            empty={
              search.q
                ? "No matching agents. Press Esc to clear."
                : "No tokens issued. Create a personal token or authorize an OAuth client."
            }
          />
        }
        detail={
          <>
            <div className="pane-heading">
              <span>
                {search.detail === "new" ? "Personal token" : "Agent access"}
              </span>
              <Button variant="ghost" onClick={back}>
                Close <span className="kbd">Esc</span>
              </Button>
            </div>
            <div className="pane-body">
              {search.detail === "new" ? (
                <TokenForm onDone={back} />
              ) : (
                selected && (
                  <>
                    <h2>{selected.name}</h2>
                    <p className="pane-meta">
                      {selected.oauth ? "OAuth" : "Personal token"} ·{" "}
                      {selected.revoked ? "Revoked" : "Issued"}
                    </p>
                    <Facts
                      items={[
                        {
                          label: "Last used",
                          value: selected.lastUsedAt
                            ? date(selected.lastUsedAt)
                            : "Never",
                        },
                        { label: "Client", value: selected.name },
                        { label: "Resource", value: selected.resource },
                        {
                          label: "Workspace",
                          value:
                            selected.resource === "/mcp-admin"
                              ? "Administration"
                              : (workspaces.find(
                                  (w) => w.id === selected.workspaceId
                                )?.name ?? "Deleted workspace"),
                        },
                      ]}
                    />
                    <div className="detail-actions">
                      <Button
                        variant="outline"
                        render={
                          <Link to="/sessions" search={{ q: selected.name }} />
                        }
                      >
                        Show sessions
                      </Button>
                      {!selected.revoked && (
                        <ConfirmAction
                          label="Revoke token"
                          description="The agent loses access on its next request. OAuth refresh access is revoked with its grant."
                          onConfirm={async () => {
                            await write(
                              `/api/tokens/${selected.id}`,
                              "DELETE",
                              undefined,
                              nothing
                            )
                            await cache.invalidateQueries({
                              queryKey: ["tokens"],
                            })
                          }}
                        />
                      )}
                    </div>
                  </>
                )
              )}
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
