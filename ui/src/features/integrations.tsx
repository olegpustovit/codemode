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
import { useCallback, useEffect, useRef, useState } from "react"
import { Link, getRouteApi } from "@tanstack/react-router"
import {
  useMutation,
  useQueryClient,
  useSuspenseQuery,
} from "@tanstack/react-query"
import { Plus, Search, Plug } from "lucide-react"
import { OAuthAppSetup } from "./oauth-app"
import {
  integrationsQuery,
  sourcesQuery,
  accountsQuery,
  oauthAppsQuery,
} from "./queries"
import type { Integration } from "./queries"
import { Facts } from "@/components/facts"
import { DataList, ListEmpty } from "@/components/data-list"
import { PaneStack } from "@/components/pane-stack"
import { TabBar } from "@/components/tab-bar"
import { ShellSlot } from "@/components/shell-slot"
import { VendorGlyph } from "@/components/vendor-glyph"
import { ConfirmAction } from "@/components/confirm-action"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select"
import { useStatus } from "@/components/status-bar"
import {
  created,
  httpUrl,
  message,
  nothing,
  write,
  bulkResult,
} from "@/lib/api"
import { z } from "zod"
const route = getRouteApi("/_authenticated/integrations")
const methodNames = {
  one_click: "One-click",
  oauth_app: "OAuth app",
  api_key: "API key",
  open: "Open",
  local: "Local",
}
export function integrationState(entry: Integration) {
  if (!entry.sourceId) return "Available"
  if (entry.state === "ready") return "Linked"
  if (entry.state === "needs_link") return "Needs link"
  if (entry.state === "error") return "Failed"
  return entry.state.replaceAll("_", " ")
}
export function filterIntegrations(
  entries: Integration[],
  q: string,
  tab: string
) {
  return entries
    .filter((entry) => {
      return (
        matches(
          q,
          entry.name,
          entry.category,
          entry.description,
          ...entry.toolNames
        ) &&
        (tab === "linked"
          ? entry.state === "ready"
          : tab === "attention"
            ? entry.state === "needs_link" || entry.state === "error"
            : true)
      )
    })
    .sort(
      (a, b) =>
        a.category.localeCompare(b.category) || a.name.localeCompare(b.name)
    )
}
export function IntegrationsPage() {
  const entries = useSuspenseQuery(integrationsQuery).data
  const search = route.useSearch()
  const navigate = route.useNavigate()
  const [cursor, setCursor] = useListCursor(
    search.selected,
    `${search.q ?? ""}:${search.tab ?? ""}`
  )
  const [pane, setPane] = useListCursor(search.detail, search.selected)
  const selection = useListSelection(`${search.tab ?? ""}:${search.q ?? ""}`)
  const cache = useQueryClient()
  const { report } = useStatus()
  const list = useRef<HTMLTableElement>(null)
  const input = useRef<HTMLInputElement>(null)
  const initialFocus = useRef(!search.selected)
  const mountSearch = useCallback((node: HTMLInputElement | null) => {
    input.current = node
    if (node && initialFocus.current) {
      initialFocus.current = false
      node.focus()
    }
  }, [])
  const q = search.q ?? ""
  const tab = search.tab ?? "all"
  const rows = filterIntegrations(entries, q, tab)
  const selected =
    rows.find((entry) => entry.slug === cursor || entry.sourceId === cursor) ??
    rows[0]
  const selectedEntries = selection.value.allMatching
    ? rows
    : rows.filter((entry) => selection.value.ids.includes(entry.slug))
  const selectedCount = selectedEntries.length
  const bulk = selectedCount > 1
  const customOnly = selectedEntries.every(
    (entry) => entry.sourceId && entry.slug.startsWith("source-")
  )
  const open = bulk || (pane === "open" && !!selected)
  const tabs = [
    { id: "all", label: "All", count: entries.length },
    {
      id: "linked",
      label: "Linked",
      count: entries.filter((entry) => entry.state === "ready").length,
    },
    {
      id: "attention",
      label: "Needs attention",
      count: entries.filter(
        (entry) => entry.state === "needs_link" || entry.state === "error"
      ).length,
    },
  ]
  const choose = (id: string, push = false) => {
    setCursor(id)
    if (!push) return
    flushSync(() => setPane("open"))
    void navigate({
      search: {
        ...search,
        selected: id,
        oauth: undefined,
        oauthError: undefined,
        detail: push || open ? "open" : undefined,
      },
      replace: !push,
    })
  }
  const changeTab = (id: string) => {
    const target = tabs.find((item) => item.id === id)
    if (!target) return
    void navigate({
      search: {
        q: search.q,
        tab: id === "linked" || id === "attention" ? id : "all",
      },
    })
  }
  const move = (delta: number) => {
    const index = rows.findIndex((entry) => entry.slug === selected?.slug)
    const next = rows[Math.max(0, Math.min(rows.length - 1, index + delta))]
    if (next) choose(next.slug)
  }
  const back = () => {
    if (selection.value.allMatching || selection.value.ids.length > 1) {
      selection.set({
        ids: selected?.slug ? [selected?.slug] : [],
        allMatching: false,
      })
      return
    }
    setPane("")
    if (q) void navigate({ search: { ...search, q: undefined }, replace: true })
    else
      void navigate({ search: { ...search, detail: undefined }, replace: true })
    list.current?.focus()
  }
  useEffect(() => {
    if (search.oauth)
      report(
        search.oauth === "linked"
          ? "Account linked; synchronizing tools"
          : (search.oauthError ??
              "Authorization cancelled. You can try again or use an API key."),
        search.oauth === "denied" ? "error" : "normal"
      )
  }, [search.oauth, search.oauthError, report])
  useEffect(() => {
    const keys = (event: KeyboardEvent) => {
      if (
        event.defaultPrevented ||
        (event.key === "Escape" &&
          event.target instanceof Element &&
          !!event.target.closest("#section-navigation")) ||
        event.ctrlKey ||
        event.altKey ||
        event.metaKey
      )
        return
      const editing =
        event.target instanceof HTMLElement &&
        event.target.closest("input,textarea,select,[contenteditable=true]")
      if (editing) {
        if (event.key === "Escape" && event.target === input.current) {
          event.preventDefault()
          back()
        }
        return
      }
      if (event.key === "/") {
        event.preventDefault()
        input.current?.focus()
      } else if (event.key === "[" || event.key === "]") {
        event.preventDefault()
        const index = tabs.findIndex((item) => item.id === tab)
        changeTab(
          tabs[
            (index + (event.key === "]" ? 1 : tabs.length - 1)) % tabs.length
          ].id
        )
      } else if (event.key === "Escape" && open) {
        event.preventDefault()
        back()
      }
    }
    document.addEventListener("keydown", keys)
    return () => document.removeEventListener("keydown", keys)
  })
  return (
    <section className="list-screen integration-screen">
      <ShellSlot target="shell-search">
        <label className="global-search">
          <Search aria-hidden="true" />
          <input
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
            ref={mountSearch}
            type="search"
            aria-describedby={/[*?]/.test(q) ? "list-search-hint" : undefined}
            aria-label="Search this list"
            placeholder="Search vendors and tools"
            value={q}
            onChange={(event) =>
              void navigate({
                search: { ...search, q: event.target.value || undefined },
                replace: true,
              })
            }
          />
          {/[*?]/.test(q) && <SearchHint />}
          <span className="kbd">/</span>
        </label>
      </ShellSlot>
      <header className="list-heading">
        <div>
          <Breadcrumbs
            items={[
              {
                label: "Integrations",
                onSelect: () =>
                  void navigate({ search: { q: search.q, tab: search.tab } }),
              },
              ...(open && selected ? [{ label: selected.name }] : []),
            ]}
          />
          <h1>Integrations</h1>
          <p>
            Connect a tool once. Every agent in the workspace can then search
            and call it.
          </p>
        </div>
        <Button render={<Link to="/sources/new" />}>
          <Plus data-icon="inline-start" />
          Add by URL
        </Button>
      </header>
      <TabBar
        tabs={tabs}
        selected={tab}
        onSelect={changeTab}
        panelId="integration-panes"
      />
      <PaneStack
        id="integration-panes"
        open={open}
        list={
          <DataList
            selection={selection}
            listRef={list}
            label="Integrations"
            columns={["Vendor", "Connects with", "Status"]}
            selectedId={selected?.slug}
            rows={rows.map((entry) => ({
              id: entry.slug,
              label: entry.name,
              group: entry.category,
              renameable: entry.slug.startsWith("source-"),
              cells: [
                <span key="name" className="vendor-name">
                  <VendorGlyph name={entry.icon} label={entry.name} />
                  <span>
                    <MatchText query={q}>{entry.name}</MatchText>
                  </span>
                </span>,
                entry.methods.map((method) => methodNames[method]).join(" / "),
                <span
                  key="state"
                  className={entry.state === "error" ? "bad" : undefined}
                >
                  {integrationState(entry)}
                  {entry.state === "ready"
                    ? ` · ${entry.toolCount} ${entry.toolCount === 1 ? "tool" : "tools"}`
                    : ""}
                </span>,
              ],
            }))}
            onRename={async (id, name) => {
              const entry = entries.find((entry) => entry.slug === id)
              if (!entry?.sourceId || !id.startsWith("source-")) return
              await write(
                `/api/sources/${entry.sourceId}`,
                "PATCH",
                { name, icon: entry.icon },
                nothing
              )
              await Promise.all([
                cache.invalidateQueries({ queryKey: ["integrations"] }),
                cache.invalidateQueries({ queryKey: ["sources"] }),
              ])
              report("Connector renamed; Python namespace unchanged")
            }}
            onAction={(key) => {
              if (
                key !== "e" ||
                !selected?.sourceId ||
                !selected.slug.startsWith("source-")
              )
                return false
              void navigate({
                to: "/sources/$sourceId",
                params: { sourceId: selected.sourceId },
              })
              return true
            }}
            onSelect={choose}
            onOpen={(id) => choose(id, true)}
            onBack={back}
            empty={
              q ? (
                <ListEmpty
                  title="No matching integrations"
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
                  icon={<Plug strokeWidth={1} aria-hidden="true" />}
                  title={
                    tab === "attention"
                      ? "No integrations need attention"
                      : "No linked integrations"
                  }
                  description={
                    tab === "attention"
                      ? "Your integrations have no connection errors to resolve."
                      : "Choose a provider or add a custom connector to make its tools available to agents."
                  }
                >
                  <Button variant="outline" onClick={() => changeTab("all")}>
                    Browse integrations
                  </Button>
                </ListEmpty>
              )
            }
          />
        }
        detail={
          bulk ? (
            <BulkPane
              key={JSON.stringify(selection.value)}
              count={selectedCount}
              names={selectedEntries.map((entry) => entry.name)}
              allMatching={selection.value.allMatching}
              actions={[
                {
                  id: "sync",
                  label: customOnly
                    ? "Sync connectors"
                    : "Select only custom connectors to sync",
                  disabled: !customOnly,
                },
              ]}
              onClose={() =>
                selection.set({
                  ids: selected ? [selected.slug] : [],
                  allMatching: false,
                })
              }
              onAction={async () => {
                const result = await write(
                  "/api/sources/bulk-sync",
                  "POST",
                  {
                    selection: {
                      ids: selectedEntries.map((entry) => entry.sourceId),
                    },
                    expectedCount: selectedCount,
                  },
                  bulkResult
                )
                await Promise.all([
                  cache.invalidateQueries({ queryKey: ["integrations"] }),
                  cache.invalidateQueries({ queryKey: ["sources"] }),
                ])
                report(
                  `Synchronization requested for ${result.changed} connectors`
                )
              }}
            />
          ) : (
            selected && (
              <IntegrationDetails
                key={selected.slug}
                entry={selected}
                close={back}
                oauthError={
                  search.oauth === "denied" ? search.oauthError : undefined
                }
              />
            )
          )
        }
      />
      <ShellSlot target="status-keys">
        <button
          onClick={() => selected && choose(selected.slug, true)}
          disabled={!selected}
        >
          <span className="kbd">↵</span>Open
        </button>
        <button
          onClick={() => {
            move(1)
            list.current?.focus()
          }}
          disabled={!rows.length}
        >
          <span className="kbd">↑↓</span>Select
        </button>
        <button
          onClick={() =>
            changeTab(
              tabs[
                (tabs.findIndex((item) => item.id === tab) + 1) % tabs.length
              ].id
            )
          }
        >
          <span className="kbd">[ ]</span>Tabs
        </button>
        <button onClick={() => input.current?.focus()}>
          <span className="kbd">/</span>Search
        </button>
        {(open || q) && (
          <button onClick={back}>
            <span className="kbd">Esc</span>Back
          </button>
        )}
      </ShellSlot>
    </section>
  )
}
function IntegrationDetails({
  entry,
  close,
  oauthError,
}: {
  entry: Integration
  close: () => void
  oauthError?: string
}) {
  const sources = useSuspenseQuery(sourcesQuery).data
  const accounts = useSuspenseQuery(accountsQuery).data
  const apps = useSuspenseQuery(oauthAppsQuery).data
  const source = sources.find((source) => source.id === entry.sourceId)
  const linked = accounts.some((account) => account.sourceId === entry.sourceId)
  const cache = useQueryClient()
  const { report } = useStatus()
  const [credential, setCredential] = useState("")
  const [method, setMethod] = useState(entry.methods[0] ?? "api_key")
  const refresh = () =>
    Promise.all(
      ["integrations", "sources", "accounts", "policies"].map((queryKey) =>
        cache.invalidateQueries({ queryKey: [queryKey] })
      )
    )
  const connect = useMutation({
    mutationFn: async () => {
      const id =
        entry.sourceId ??
        (
          await write(
            `/api/integrations/${entry.slug}`,
            "POST",
            undefined,
            created
          )
        ).id
      if (method === "api_key")
        await write(`/api/accounts/${id}`, "PUT", { credential }, nothing)
      else if (method === "one_click" || method === "oauth_app")
        return (
          await write(
            `/api/accounts/${id}/oauth`,
            "POST",
            undefined,
            z.object({ authorizationUrl: httpUrl })
          )
        ).authorizationUrl
      return undefined
    },
    onSuccess: async (authorizationUrl) => {
      setCredential("")
      await refresh()
      report(
        authorizationUrl
          ? "Continue at the provider to authorize access"
          : "Connection saved; synchronizing tools"
      )
      if (authorizationUrl) window.location.assign(authorizationUrl)
    },
    onError: (error) => report(message(error), "error"),
  })
  const id = entry.sourceId
  const managed = source && source.kind !== "mcp_http"
  return (
    <>
      <header className="pane-heading">
        <span>{entry.category}</span>
        <Button variant="ghost" onClick={close}>
          Close <span className="kbd">Esc</span>
        </Button>
      </header>
      <div className="pane-body">
        <h2>{entry.name}</h2>
        <p className="pane-meta">{integrationState(entry)}</p>
        <VendorGlyph name={entry.icon} label={entry.name} large />
        <p>{entry.description}</p>
        <Facts
          items={[
            {
              label: "Connection",
              value: entry.methods
                .map((method) => methodNames[method])
                .join(" / "),
            },
            {
              label: "Tools",
              value: <code>{linked ? entry.toolCount : "—"}</code>,
            },
            ...(source
              ? [
                  {
                    label: "Namespace",
                    value: <code>tools.{source.pythonName}</code>,
                  },
                ]
              : []),
          ]}
        />
        {(entry.error || connect.error || oauthError) && (
          <p className="connection-error" role="alert">
            {connect.error
              ? message(connect.error)
              : (oauthError ?? entry.error)}
          </p>
        )}
        {entry.methods.length > 1 && (
          <div className="credential-field">
            <label htmlFor="integration-method">Connection method</label>
            <NativeSelect
              id="integration-method"
              value={method}
              disabled={connect.isPending}
              onChange={(event) => {
                const next = entry.methods.find(
                  (method) => method === event.target.value
                )
                if (next) {
                  setMethod(next)
                  setCredential("")
                }
              }}
            >
              {entry.methods.map((method) => (
                <NativeSelectOption key={method} value={method}>
                  {methodNames[method]}
                </NativeSelectOption>
              ))}
            </NativeSelect>
          </div>
        )}
        {method === "oauth_app" && (
          <OAuthAppSetup entry={entry} configured={apps.includes(entry.slug)} />
        )}
        {managed ? (
          <Button
            render={
              <Link
                to="/sources/$sourceId/accounts"
                params={{ sourceId: source.id }}
              />
            }
          >
            Manage credentials
          </Button>
        ) : (
          <form
            className="connection-form"
            onSubmit={(event) => {
              event.preventDefault()
              connect.mutate()
            }}
          >
            {method === "api_key" && (
              <div className="credential-field">
                <label htmlFor="integration-key">
                  {linked ? "Replace API key" : "API key"}
                </label>
                <Input
                  id="integration-key"
                  aria-describedby="integration-key-hint"
                  type="password"
                  autoComplete="off"
                  required
                  minLength={1}
                  maxLength={65536}
                  value={credential}
                  onChange={(event) => setCredential(event.target.value)}
                  disabled={connect.isPending}
                />
                <span id="integration-key-hint" className="form-hint">
                  Sealed in your tenant. It is never shown again.
                </span>
              </div>
            )}
            {(!id || method !== "open") &&
              (method !== "oauth_app" || apps.includes(entry.slug)) && (
                <Button type="submit" disabled={connect.isPending}>
                  {connect.isPending
                    ? "Connecting…"
                    : method === "one_click" || method === "oauth_app"
                      ? linked
                        ? "Reconnect"
                        : "Connect"
                      : method === "open"
                        ? "Add integration"
                        : "Save API key"}
                </Button>
              )}
          </form>
        )}
        <div className="detail-actions">
          {entry.docs && (
            <a href={entry.docs} target="_blank" rel="noreferrer">
              Provider documentation
            </a>
          )}
          {id && (
            <Link to="/sources/$sourceId" params={{ sourceId: id }}>
              Connection settings
            </Link>
          )}
          {id && linked && (
            <ConfirmAction
              label="Unlink account"
              description="Agents lose access until you link this account again."
              onConfirm={async () => {
                await write(`/api/accounts/${id}`, "DELETE", undefined, nothing)
                await refresh()
              }}
            />
          )}
        </div>
      </div>
    </>
  )
}
