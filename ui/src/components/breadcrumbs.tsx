import { Link } from "@tanstack/react-router"
import { useSuspenseQuery } from "@tanstack/react-query"
import { workspacesQuery } from "@/features/queries"
import { useWorkspace } from "./workspace-context"
export function Breadcrumbs({
  items,
  workspaceId,
}: {
  items: { label: string; onSelect?: () => void }[]
  workspaceId?: string
}) {
  const active = useWorkspace()
  const workspaces = useSuspenseQuery(workspacesQuery).data
  const workspace = workspaces.find((w) => w.id === (workspaceId ?? active))
  return (
    <nav className="list-crumbs" aria-label="Breadcrumb">
      <Link to="/workspaces" search={{ workspace: workspace?.id }}>
        {workspace?.name ?? "All sources"}
      </Link>
      {items.map((item, index) => (
        <span className="crumb-part" key={index}>
          <span aria-hidden="true">/</span>
          {item.onSelect ? (
            <button onClick={item.onSelect}>{item.label}</button>
          ) : (
            <span
              aria-current={index === items.length - 1 ? "page" : undefined}
            >
              {item.label}
            </span>
          )}
        </span>
      ))}
    </nav>
  )
}
