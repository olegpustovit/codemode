import { relativeTime } from "@/lib/time"
import { move as transitionPane } from "@/lib/motion"
import { BulkPane } from "@/components/bulk-pane"
import { useStatus } from "@/components/status-bar"
import { matches } from "@/lib/match"
import { MatchText, SearchHint } from "@/components/match-text"
import {
  useListCursor,
  useListSelection,
  listSearchKeys,
} from "@/hooks/use-list-navigation"
import { Breadcrumbs } from "@/components/breadcrumbs"
import { useEffect, useRef, useState } from "react"
import { Link, getRouteApi } from "@tanstack/react-router"
import { useQueryClient, useSuspenseQuery } from "@tanstack/react-query"
import { Search, KeyRound } from "lucide-react"
import { tokensQuery, workspacesQuery } from "./queries"
import { TokenForm } from "./tokens"
import { DataList, ListEmpty } from "@/components/data-list"
import { PaneStack } from "@/components/pane-stack"
import { TabBar } from "@/components/tab-bar"
import { ShellSlot } from "@/components/shell-slot"
import { Facts } from "@/components/facts"
import { ConfirmAction } from "@/components/confirm-action"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { date } from "@/components/common"
import { nothing, write, bulkResult } from "@/lib/api"
const route = getRouteApi("/_authenticated/tokens")
export function TokensPage() {
  const search = route.useSearch()
  const navigate = route.useNavigate()
  const [cursor, setCursor] = useListCursor(search.selected, search.q)
  const [pane, setPane] = useListCursor(search.detail, search.selected)
  const selection = useListSelection(search.q ?? "")
  const tokens = useSuspenseQuery(tokensQuery).data
  const workspaces = useSuspenseQuery(workspacesQuery).data
  const rows = tokens.filter((t) => matches(search.q ?? "", t.name))
  const selected = rows.find((t) => t.id === cursor) ?? rows[0]
  const list = useRef<HTMLTableElement>(null)
  const input = useRef<HTMLInputElement>(null)
  const cache = useQueryClient()
  const { report } = useStatus()
  const [revoking, setRevoking] = useState(false)
  const selectedTokens = selection.value.allMatching
    ? rows
    : rows.filter((token) => selection.value.ids.includes(token.id))
  const selectedCount = selectedTokens.length || (selected ? 1 : 0)
  const bulk = selectedTokens.length > 1 || revoking
  const revokeSelection = selectedTokens.length
    ? selectedTokens
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
    setRevoking(false)
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
  const choose = (id: string, open = false) => {
    setCursor(id)
    if (!open) return
    transitionPane(pane === "open" ? "swap" : "open", () => {
      setPane("open")
      void navigate({
        search: {
          ...search,
          selected: id,
          detail: open || search.detail === "open" ? "open" : undefined,
        },
        replace: !open,
      })
    })
  }
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
            aria-describedby={
              /[*?]/.test(search.q ?? "") ? "list-search-hint" : undefined
            }
            aria-label="Search agents"
            placeholder="Search agents"
            value={search.q ?? ""}
            onChange={(e) =>
              void navigate({
                search: { q: e.target.value || undefined },
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
            items={[
              { label: "Tokens", onSelect: back },
              ...(pane === "new"
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
        open={bulk || pane === "new" || (pane === "open" && !!selected)}
        list={
          <DataList
            selection={selection}
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
                  <MatchText key="name" query={search.q ?? ""}>
                    {t.name}
                  </MatchText>,
                  t.admin
                    ? "Admin tools"
                    : t.oauth
                      ? "OAuth"
                      : "Personal token",
                  workspaces.find((w) => w.id === t.workspaceId)?.name ??
                    "Deleted workspace",
                  t.lastUsedAt ? (
                    <time
                      key="used"
                      dateTime={t.lastUsedAt}
                      title={date(t.lastUsedAt)}
                    >
                      {relativeTime(t.lastUsedAt)}
                    </time>
                  ) : (
                    "Never"
                  ),
                ],
              }))
              .sort((a, b) => a.group.localeCompare(b.group))}
            onDelete={() => setRevoking(true)}
            onSelect={choose}
            onOpen={(id) => choose(id, true)}
            onBack={back}
            empty={
              search.q ? (
                <ListEmpty
                  title="No matching agents"
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
                  icon={<KeyRound strokeWidth={1} aria-hidden="true" />}
                  title="No agent access yet"
                  description="Create a personal token for a headless agent, or connect a client with OAuth."
                >
                  <Button
                    onClick={() => {
                      setPane("new")
                      void navigate({ search: { detail: "new" } })
                    }}
                  >
                    New personal token
                  </Button>
                </ListEmpty>
              )
            }
          />
        }
        detail={
          bulk ? (
            <BulkPane
              key={JSON.stringify([selection.value, revoking])}
              count={selectedCount}
              names={revokeSelection.map((token) => token.name)}
              allMatching={selection.value.allMatching}
              confirmAction={revoking ? "revoke" : undefined}
              actions={[
                {
                  id: "revoke",
                  label: "Revoke tokens",
                  confirm:
                    "These agents lose access immediately. Revoking OAuth access also revokes its refresh grant and sibling access tokens.",
                },
              ]}
              onClose={() => {
                setRevoking(false)
                selection.set({
                  ids: selected ? [selected.id] : [],
                  allMatching: false,
                })
              }}
              onAction={async () => {
                const result = await write(
                  "/api/tokens/bulk-revoke",
                  "POST",
                  {
                    selection: selection.value.allMatching
                      ? { query: search.q ?? "" }
                      : { ids: revokeSelection.map((token) => token.id) },
                    expectedCount: selectedCount,
                  },
                  bulkResult
                )
                selection.set({ ids: [], allMatching: false })
                setRevoking(false)
                setPane("")
                void navigate({
                  search: { ...search, selected: undefined, detail: undefined },
                  replace: true,
                })
                requestAnimationFrame(() => list.current?.focus())
                await cache.invalidateQueries({ queryKey: ["tokens"] })
                report(`${result.changed} access tokens revoked`)
              }}
            />
          ) : (
            <>
              <div className="pane-heading">
                <span>
                  {pane === "new" ? "Personal token" : "Agent access"}
                </span>
                <Button variant="ghost" onClick={back}>
                  Close <span className="kbd">Esc</span>
                </Button>
              </div>
              <div className="pane-body">
                {pane === "new" ? (
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
                              workspaces.find(
                                (w) => w.id === selected.workspaceId
                              )?.name ?? "Deleted workspace",
                          },
                        ]}
                      />
                      <div className="detail-actions">
                        <Button
                          variant="outline"
                          render={
                            <Link
                              to="/sessions"
                              search={{ q: selected.name }}
                            />
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
