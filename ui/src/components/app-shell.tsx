import { Menu } from "@base-ui/react/menu"
import { mcpUrl } from "@/lib/workspace-url"
import { navigationMotion } from "@/lib/motion"
import { useEffect, useState } from "react"
import {
  Link,
  Outlet,
  useMatchRoute,
  useRouter,
  useSearch,
  useRouterState,
} from "@tanstack/react-router"
import {
  useMutation,
  useQuery,
  useQueryClient,
  useSuspenseQuery,
} from "@tanstack/react-query"
import { ChevronDown, PanelLeftClose } from "lucide-react"
import { AccountMenu } from "@/components/account-menu"
import { Button } from "@/components/ui/button"
import { ErrorNotice } from "@/components/common"
import { ShellTargets } from "@/components/shell-slot"
import { StatusBar, useStatus } from "@/components/status-bar"
import { configQuery, sessionQuery } from "@/lib/runtime"
import { nothing, write } from "@/lib/api"
import { WorkspaceContext } from "./workspace-context"
import {
  workspacesQuery,
  integrationsQuery,
  tokensQuery,
  agentSessionsQuery,
  scriptsQuery,
} from "@/features/queries"
const sections = [
  {
    to: "/sessions",
    label: "Sessions",
    path: "M3 8c3-3 6 3 9 0s6-3 9 0M3 13c3-3 6 3 9 0s6-3 9 0M3 18c3-3 6 3 9 0s6-3 9 0",
  },
  {
    to: "/integrations",
    label: "Integrations",
    path: "M9 3v5M15 3v5M6 8h12v3a6 6 0 0 1-12 0zM12 17v4",
  },
  {
    to: "/workspaces",
    label: "Workspaces",
    path: "M12 3l9 5-9 5-9-5zM3 13l9 5 9-5M3 17.5l9 5 9-5",
  },
  {
    to: "/scripts",
    label: "Scripts",
    path: "M8 6l-5 6 5 6M16 6l5 6-5 6M14 4l-4 16",
  },
  {
    to: "/tokens",
    label: "Tokens",
    path: "M12 15a4 4 0 1 1-8 0a4 4 0 0 1 8 0M11 12l9-9M17 6l3 3M14 9l2 2",
  },
] as const
function sectionTransition(from: number, to: number) {
  return from === to ? false : navigationMotion(to > from ? "down" : "up")
}
export function AppShell() {
  const config = useSuspenseQuery(configQuery).data
  const session = useSuspenseQuery(sessionQuery).data
  const workspaces = useSuspenseQuery(workspacesQuery).data
  const search = useSearch({ strict: false })
  const path = useRouterState({ select: (state) => state.location.pathname })
  const [railCollapsed, setRailCollapsed] = useState(() => {
    try {
      return localStorage.getItem("codemode.rail") === "collapsed"
    } catch {
      return false
    }
  })
  useEffect(() => {
    try {
      localStorage.setItem(
        "codemode.rail",
        railCollapsed ? "collapsed" : "expanded"
      )
    } catch {
      // The rail still works when storage is disabled.
    }
  }, [railCollapsed])
  const workspace =
    workspaces.find((w) => w.id === search.workspace) ??
    workspaces.find((w) => w.allSources) ??
    workspaces[0]
  const integrations = useQuery(integrationsQuery).data
  const tokens = useQuery(tokensQuery).data
  const sessions = useQuery(agentSessionsQuery()).data
  const scripts = useQuery({
    ...scriptsQuery(workspace?.id ?? ""),
    enabled: !!workspace,
  }).data
  const counts = {
    "/sessions": sessions ? String(sessions.totalMatches) : "—",
    "/integrations":
      integrations?.filter((s) => s.state === "ready").length ?? "—",
    "/workspaces": workspaces.length,
    "/scripts": scripts?.length ?? "—",
    "/tokens": tokens?.filter((t) => !t.revoked).length ?? "—",
  }
  const cache = useQueryClient()
  const router = useRouter()
  const match = useMatchRoute()
  const { report } = useStatus()
  const [searchHost, setSearchHost] = useState<HTMLDivElement | null>(null)
  const [statusHost, setStatusHost] = useState<HTMLDivElement | null>(null)
  function isActive(to: (typeof sections)[number]["to"]) {
    return to === "/scripts"
      ? path.includes("/scripts")
      : to === "/workspaces"
        ? path.startsWith("/workspaces") && !path.includes("/scripts")
        : Boolean(match({ to, fuzzy: true }))
  }
  const currentSection = sections.findIndex(({ to }) => isActive(to))
  useEffect(() => {
    const keys = (event: KeyboardEvent) => {
      if (
        event.defaultPrevented ||
        event.ctrlKey ||
        event.metaKey ||
        event.altKey ||
        (event.target instanceof Element &&
          event.target.closest("input,textarea,select,[contenteditable=true]"))
      )
        return
      if (
        event.key === "\\" &&
        !window.matchMedia("(max-width: 767px)").matches
      ) {
        event.preventDefault()
        setRailCollapsed((collapsed) => !collapsed)
        return
      }
      const section = sections[Number(event.key) - 1]
      if (section) {
        event.preventDefault()
        void router.navigate({
          to: section.to,
          viewTransition: sectionTransition(
            currentSection,
            sections.indexOf(section)
          ),
          search: { workspace: workspace?.id },
        })
      }
    }
    document.addEventListener("keydown", keys)
    return () => document.removeEventListener("keydown", keys)
  }, [router, workspace?.id, currentSection])
  const chooseWorkspace = (id: string) => {
    if (path === "/scripts" || path === "/workspaces") {
      void router.navigate({
        to: path,
        search: { workspace: id },
        viewTransition: navigationMotion("ws"),
      })
    } else {
      const url = new URL(router.state.location.href, window.location.origin)
      url.searchParams.set("workspace", id)
      void router.navigate({
        href: url.pathname + url.search + url.hash,
        viewTransition: navigationMotion("ws"),
      })
    }
    report(
      `Workspace: ${workspaces.find((w) => w.id === id)?.name ?? "All sources"}`
    )
  }
  const logout = useMutation({
    mutationFn: () => write("/api/session", "DELETE", undefined, nothing),
    onSuccess: async () => {
      cache.clear()
      await router.navigate({ to: "/login", search: { redirect: "/sources" } })
    },
  })
  return (
    <WorkspaceContext.Provider value={workspace?.id}>
      <ShellTargets.Provider
        value={{ "shell-search": searchHost, "status-keys": statusHost }}
      >
        <div
          className="app-shell"
          data-rail={railCollapsed ? "collapsed" : "expanded"}
        >
          <a className="skip-link" href="#content">
            Skip to content
          </a>
          <header className="shell-top">
            <Link to="/sources" className="shell-brand" aria-label="Codemode">
              <i aria-hidden="true" />
              <span>Codemode</span>
            </Link>
            <Menu.Root>
              <Menu.Trigger
                className="workspace-switch"
                aria-label="Switch workspace"
              >
                Workspace <b>{workspace?.name}</b>
                <ChevronDown aria-hidden="true" />
              </Menu.Trigger>
              <Menu.Portal>
                <Menu.Positioner
                  align="start"
                  sideOffset={6}
                  className="workspace-menu-positioner"
                >
                  <Menu.Popup
                    className="workspace-menu"
                    aria-label="Workspaces"
                  >
                    <div className="workspace-menu-heading">
                      Switch workspace
                    </div>
                    <Menu.RadioGroup
                      value={workspace?.id ?? ""}
                      onValueChange={chooseWorkspace}
                    >
                      {workspaces.map((item) => (
                        <Menu.RadioItem
                          key={item.id}
                          value={item.id}
                          className="workspace-menu-item"
                        >
                          <span>{item.name}</span>
                          <code>
                            {
                              mcpUrl(config.hostname, item.id).split(
                                config.hostname
                              )[1]
                            }
                          </code>
                          <small>
                            {item.allSources
                              ? "All sources"
                              : `${item.sourceIds.length} sources`}
                          </small>
                        </Menu.RadioItem>
                      ))}
                    </Menu.RadioGroup>
                    <Menu.Separator className="workspace-menu-divider" />
                    <div className="workspace-menu-footer">
                      <Menu.Item render={<Link to="/workspaces/new" />}>
                        New workspace
                      </Menu.Item>
                      {workspace && (
                        <Menu.Item
                          render={
                            <Link
                              to="/workspaces/$workspaceId"
                              params={{ workspaceId: workspace.id }}
                            />
                          }
                        >
                          Manage {workspace.name}
                        </Menu.Item>
                      )}
                    </div>
                  </Menu.Popup>
                </Menu.Positioner>
              </Menu.Portal>
            </Menu.Root>
            <div
              id="shell-search"
              className="shell-search"
              ref={setSearchHost}
            />
            <Button
              variant="outline"
              className="agent-url"
              onClick={() => {
                void navigator.clipboard
                  .writeText(mcpUrl(config.hostname, workspace?.id))
                  .then(() => report("MCP URL copied"))
                  .catch(() =>
                    report(
                      `Copy failed. Select and copy ${mcpUrl(config.hostname, workspace?.id)}`,
                      "error"
                    )
                  )
              }}
            >
              Copy MCP URL
            </Button>
            <AccountMenu
              email={session.user?.email ?? ""}
              onLogout={() => logout.mutate()}
              busy={logout.isPending}
            />
            <ErrorNotice error={logout.error} />
          </header>
          <aside className="section-rail" id="section-navigation">
            <nav className="section-nav" aria-label="Sections">
              {sections.map((section) => (
                <Link
                  key={section.to}
                  to={section.to}
                  search={{ workspace: workspace?.id }}
                  className="section-link"
                  aria-label={section.label}
                  title={section.label}
                  aria-current={isActive(section.to) ? "page" : undefined}
                  viewTransition={sectionTransition(
                    currentSection,
                    sections.indexOf(section)
                  )}
                >
                  {isActive(section.to) && (
                    <span className="rail-mark" aria-hidden="true" />
                  )}
                  <svg
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    aria-hidden="true"
                  >
                    <path d={section.path} />
                  </svg>
                  <span className="section-label">{section.label}</span>
                  <span className="section-count" aria-hidden="true">
                    {counts[section.to]}
                  </span>
                </Link>
              ))}
            </nav>
            <fieldset className="rail-workspaces" aria-label="Choose workspace">
              <p>Workspaces</p>
              {workspaces.map((w) => (
                <button
                  key={w.id}
                  aria-pressed={workspace?.id === w.id}
                  aria-label={w.name}
                  title={w.name}
                  onClick={() => {
                    chooseWorkspace(w.id)
                  }}
                >
                  <span className="workspace-label">{w.name}</span>
                  <span className="workspace-initials" aria-hidden="true">
                    {w.name
                      .trim()
                      .split(/\s+/)
                      .slice(0, 2)
                      .map((word) => word[0])
                      .join("")
                      .toUpperCase()}
                  </span>
                  {workspace?.id === w.id && (
                    <span className="workspace-check" aria-hidden="true">
                      ✓
                    </span>
                  )}
                </button>
              ))}
            </fieldset>
            <button
              type="button"
              className="rail-collapse"
              aria-expanded={!railCollapsed}
              aria-controls="section-navigation"
              aria-label={railCollapsed ? "Expand sidebar" : "Collapse sidebar"}
              title={railCollapsed ? "Expand sidebar" : "Collapse sidebar"}
              onClick={() => setRailCollapsed(!railCollapsed)}
            >
              <PanelLeftClose aria-hidden="true" />
              <span>Collapse</span>
              <kbd className="kbd">{"\\"}</kbd>
            </button>
          </aside>
          <main id="content" className="work-area" tabIndex={-1}>
            <Outlet />
          </main>
          <StatusBar keysRef={setStatusHost}>
            <span className="rail-key">
              <kbd className="kbd">{"\\"}</kbd> Sidebar
            </span>
          </StatusBar>
        </div>
      </ShellTargets.Provider>
    </WorkspaceContext.Provider>
  )
}
