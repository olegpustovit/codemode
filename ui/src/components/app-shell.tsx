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
import { ChevronDown } from "lucide-react"
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
export function AppShell() {
  const config = useSuspenseQuery(configQuery).data
  const session = useSuspenseQuery(sessionQuery).data
  const workspaces = useSuspenseQuery(workspacesQuery).data
  const search = useSearch({ strict: false })
  const path = useRouterState({ select: (state) => state.location.pathname })
  const [pickedWorkspace, setPickedWorkspace] = useState<string>()
  const workspace =
    workspaces.find((w) => w.id === (search.workspace ?? pickedWorkspace)) ??
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
      const section = sections[Number(event.key) - 1]
      if (section) {
        event.preventDefault()
        setPickedWorkspace(workspace?.id)
        void router.navigate({
          to: section.to,
          search:
            section.to === "/scripts" || section.to === "/workspaces"
              ? { workspace: workspace?.id }
              : undefined,
        })
      }
    }
    document.addEventListener("keydown", keys)
    return () => document.removeEventListener("keydown", keys)
  }, [router, workspace?.id])
  const chooseWorkspace = (id: string) => {
    setPickedWorkspace(id)
    if (path === "/scripts")
      void router.navigate({ to: "/scripts", search: { workspace: id } })
    else if (path.startsWith("/workspaces"))
      void router.navigate({ to: "/workspaces", search: { workspace: id } })
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
        <div className="app-shell">
          <a className="skip-link" href="#content">
            Skip to content
          </a>
          <header className="shell-top">
            <Link to="/sources" className="shell-brand">
              <i aria-hidden="true" />
              Codemode
            </Link>
            <button
              className="workspace-switch"
              aria-label="Switch workspace"
              title="Switch workspace"
              onClick={() => {
                const index = workspaces.findIndex(
                  (w) => w.id === workspace?.id
                )
                const next = workspaces[(index + 1) % workspaces.length]
                if (next) chooseWorkspace(next.id)
              }}
            >
              Workspace <b>{workspace?.name}</b>
              <ChevronDown aria-hidden="true" />
            </button>
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
                  .writeText(`https://${config.hostname}/mcp`)
                  .then(() => report("Agent URL copied"))
                  .catch(() =>
                    report(
                      `Copy failed. Select and copy https://${config.hostname}/mcp`,
                      "error"
                    )
                  )
              }}
            >
              Copy agent URL
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
                  search={
                    section.to === "/scripts" || section.to === "/workspaces"
                      ? { workspace: workspace?.id }
                      : undefined
                  }
                  className="section-link"
                  aria-label={section.label}
                  aria-current={
                    (
                      section.to === "/scripts"
                        ? path.includes("/scripts")
                        : section.to === "/workspaces"
                          ? path.startsWith("/workspaces") &&
                            !path.includes("/scripts")
                          : match({ to: section.to, fuzzy: true })
                    )
                      ? "page"
                      : undefined
                  }
                  onClick={() => {
                    setPickedWorkspace(workspace?.id)
                  }}
                >
                  <svg
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    aria-hidden="true"
                  >
                    <path d={section.path} />
                  </svg>
                  <span>{section.label}</span>
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
                  onClick={() => {
                    chooseWorkspace(w.id)
                  }}
                >
                  <span>{w.name}</span>
                  {workspace?.id === w.id && (
                    <span className="workspace-check" aria-hidden="true">
                      ✓
                    </span>
                  )}
                </button>
              ))}
            </fieldset>
          </aside>
          <main id="content" className="work-area" tabIndex={-1}>
            <Outlet />
          </main>
          <StatusBar keysRef={setStatusHost} />
        </div>
      </ShellTargets.Provider>
    </WorkspaceContext.Provider>
  )
}
