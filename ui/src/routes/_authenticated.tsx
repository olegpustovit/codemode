import {
  createFileRoute,
  Link,
  Outlet,
  redirect,
  useMatchRoute,
  useRouter,
} from "@tanstack/react-router"
import {
  useMutation,
  useQueryClient,
  useSuspenseQuery,
} from "@tanstack/react-query"
import {
  Plug,
  Network,
  KeyRound,
  ListChecks,
  ShieldCheck,
  Users,
  LogOut,
  Boxes,
} from "lucide-react"
import {
  Sidebar,
  SidebarProvider,
  SidebarHeader,
  SidebarContent,
  SidebarMenu,
  SidebarMenuItem,
  SidebarMenuButton,
  SidebarFooter,
  SidebarInset,
  SidebarTrigger,
  useSidebar,
} from "@/components/ui/sidebar"
import { Button } from "@/components/ui/button"
import { NotFound, Pending, RouteError, ErrorNotice } from "@/components/common"
import { configQuery, sessionQuery } from "@/lib/runtime"
import { nothing, safeReturnPath, write } from "@/lib/api"
const items = [
  { to: "/sources", label: "Sources", icon: Plug },
  { to: "/accounts", label: "Accounts", icon: Users },
  { to: "/workspaces", label: "Workspaces", icon: Network },
  { to: "/approvals", label: "Approvals", icon: ShieldCheck },
  { to: "/executions", label: "Executions", icon: ListChecks },
  { to: "/tokens", label: "Access tokens", icon: KeyRound },
] as const
export const Route = createFileRoute("/_authenticated")({
  beforeLoad: async ({ context, location }) => {
    const session = await context.queryClient.query(sessionQuery)
    if (!session.authenticated)
      throw redirect({
        to: "/login",
        search: { redirect: safeReturnPath(location.href) },
      })
  },
  component: Shell,
  pendingComponent: Pending,
  errorComponent: RouteError,
  notFoundComponent: NotFound,
})
function Shell() {
  const config = useSuspenseQuery(configQuery).data
  const session = useSuspenseQuery(sessionQuery).data
  const cache = useQueryClient()
  const router = useRouter()
  const logout = useMutation({
    mutationFn: () => write("/api/session", "DELETE", undefined, nothing),
    onSuccess: async () => {
      cache.clear()
      await router.navigate({ to: "/login", search: { redirect: "/sources" } })
    },
  })
  return (
    <SidebarProvider>
      <a className="skip-link" href="#content">
        Skip to content
      </a>
      <Sidebar>
        <SidebarHeader>
          <Link to="/sources" className="brand">
            <Boxes aria-hidden="true" />
            Codemode
          </Link>
          <p className="sidebar-domain">{config.hostname}</p>
        </SidebarHeader>
        <SidebarContent>
          <Navigation />
        </SidebarContent>
        <SidebarFooter>
          <p className="px-2 text-sm break-all text-muted-foreground">
            {session.user?.email}
          </p>
          <ErrorNotice error={logout.error} />
          <Button
            variant="ghost"
            onClick={() => logout.mutate()}
            disabled={logout.isPending}
          >
            <LogOut data-icon="inline-start" />
            Sign out
          </Button>
        </SidebarFooter>
      </Sidebar>
      <SidebarInset>
        <header className="app-bar">
          <SidebarTrigger />
          <span>Control plane</span>
        </header>
        <main id="content" className="work-area">
          <Outlet />
        </main>
      </SidebarInset>
    </SidebarProvider>
  )
}
function Navigation() {
  const match = useMatchRoute()
  const { setOpenMobile } = useSidebar()
  return (
    <SidebarMenu>
      {items.map((item) => (
        <SidebarMenuItem key={item.to}>
          <SidebarMenuButton
            render={<Link to={item.to} />}
            isActive={!!match({ to: item.to, fuzzy: true })}
            onClick={() => setOpenMobile(false)}
          >
            <item.icon aria-hidden="true" />
            <span>{item.label}</span>
          </SidebarMenuButton>
        </SidebarMenuItem>
      ))}
    </SidebarMenu>
  )
}
