import { useState } from "react"
import { Link, getRouteApi, useRouter } from "@tanstack/react-router"
import {
  useMutation,
  useQueryClient,
  useSuspenseQuery,
} from "@tanstack/react-query"
import { z } from "zod"
import { ArrowLeft } from "lucide-react"
import { workspaceQuery, sourcesQuery } from "./queries"
import type { Workspace } from "./queries"
import { configQuery } from "@/lib/runtime"
import { write, created, nothing } from "@/lib/api"
import { PageHeader, StateBadge, ErrorNotice } from "@/components/common"
import { ConfirmAction } from "@/components/confirm-action"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Checkbox } from "@/components/ui/checkbox"
import {
  Field,
  FieldGroup,
  FieldLabel,
  FieldDescription,
  FieldSet,
  FieldLegend,
} from "@/components/ui/field"
const detailRoute = getRouteApi("/_authenticated/workspaces/$workspaceId")
export function NewWorkspacePage() {
  return (
    <>
      <PageHeader
        title="Create workspace"
        description="Select its sources. An empty selection exposes no tools."
      />
      <WorkspaceEditor />
    </>
  )
}
export function WorkspacePage() {
  const { workspaceId } = detailRoute.useParams()
  const workspace = useSuspenseQuery(workspaceQuery(workspaceId)).data
  const config = useSuspenseQuery(configQuery).data
  const cache = useQueryClient()
  const router = useRouter()
  return (
    <>
      <Link
        to="/workspaces"
        className="flex items-center gap-2 text-sm text-muted-foreground"
      >
        <ArrowLeft className="size-4" />
        Workspaces
      </Link>
      <PageHeader
        title={workspace.name}
        description={`https://${config.hostname}/mcp`}
      />
      <div className="record-grid">
        <WorkspaceEditor workspace={workspace} key={workspace.id} />
        <aside className="flex flex-col gap-5">
          <section className="flex flex-col gap-3">
            <h2>Agent access</h2>
            <p className="text-sm text-muted-foreground">
              Each token is bound to this workspace. Changes to its sources and
              tool policies apply to existing agents.
            </p>
            <Button
              variant="outline"
              render={
                <Link
                  to="/workspaces/$workspaceId/policies"
                  params={{ workspaceId }}
                />
              }
            >
              Manage tool policies
            </Button>
            <Button
              variant="outline"
              render={
                <Link
                  to="/workspaces/$workspaceId/scripts"
                  params={{ workspaceId }}
                />
              }
            >
              Saved scripts
            </Button>
            <Button variant="outline" render={<Link to="/tokens" />}>
              Manage access tokens
            </Button>
          </section>
          {!workspace.allSources && (
            <ConfirmAction
              label="Delete workspace"
              description="Its tokens will be revoked. Connected sources and execution history will be preserved."
              onConfirm={async () => {
                await write(
                  `/api/workspaces/${workspaceId}`,
                  "DELETE",
                  undefined,
                  nothing
                )
                await Promise.all([
                  cache.invalidateQueries({ queryKey: ["workspaces"] }),
                  cache.invalidateQueries({ queryKey: ["tokens"] }),
                ])
                await router.navigate({ to: "/workspaces" })
              }}
            />
          )}
        </aside>
      </div>
    </>
  )
}
function WorkspaceEditor({ workspace }: { workspace?: Workspace }) {
  const sources = useSuspenseQuery(sourcesQuery).data
  const [selected, setSelected] = useState(workspace?.sourceIds ?? [])
  const cache = useQueryClient()
  const router = useRouter()
  const save = useMutation({
    mutationFn: async (form: FormData) => {
      const name = z.string().trim().min(1).max(80).parse(form.get("name"))
      if (workspace) {
        await write(
          `/api/workspaces/${workspace.id}`,
          "PUT",
          { name, sourceIds: selected },
          nothing
        )
        return workspace.id
      }
      return (
        await write(
          "/api/workspaces",
          "POST",
          { name, sourceIds: selected },
          created
        )
      ).id
    },
    onSuccess: async (id) => {
      await Promise.all([
        cache.invalidateQueries({ queryKey: ["workspaces"] }),
        cache.invalidateQueries({ queryKey: ["policies", id] }),
      ])
      await router.navigate({
        to: "/workspaces/$workspaceId",
        params: { workspaceId: id },
      })
    },
  })
  const active = workspace?.allSources
    ? sources
    : sources.filter((source) => selected.includes(source.id))
  return (
    <form
      className="flex flex-col gap-5"
      onSubmit={(event) => {
        event.preventDefault()
        save.mutate(new FormData(event.currentTarget))
      }}
    >
      <FieldGroup>
        <Field>
          <FieldLabel htmlFor="workspace-name">Workspace name</FieldLabel>
          <Input
            id="workspace-name"
            name="name"
            required
            maxLength={80}
            defaultValue={workspace?.name}
            disabled={workspace?.allSources}
            placeholder="Work projects"
          />
          <FieldDescription>
            A name you recognize when connecting an agent.
          </FieldDescription>
        </Field>
        <FieldSet>
          <FieldLegend>
            {workspace?.allSources
              ? "All sources are included"
              : "Included sources"}
          </FieldLegend>
          <FieldDescription>
            {workspace?.allSources
              ? "The main workspace always includes every source."
              : "Selection changes apply to search and every tool dispatch."}
          </FieldDescription>
          {sources.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              No sources yet. Add one from the Sources page.
            </p>
          ) : (
            sources.map((source) => (
              <Field key={source.id} orientation="horizontal">
                <Checkbox
                  id={`include-${source.id}`}
                  checked={
                    !!workspace?.allSources || selected.includes(source.id)
                  }
                  disabled={workspace?.allSources}
                  onCheckedChange={(checked) =>
                    setSelected(
                      checked
                        ? [...selected, source.id]
                        : selected.filter((id) => id !== source.id)
                    )
                  }
                />
                <FieldLabel
                  htmlFor={`include-${source.id}`}
                  className="flex flex-1 items-center justify-between gap-3"
                >
                  {source.name}
                  <StateBadge state={source.status} />
                </FieldLabel>
              </Field>
            ))
          )}
        </FieldSet>
        <section aria-live="polite" className="code-panel">
          <h2>Current scope</h2>
          <p className="mt-2 text-sm">
            {active.length === 0
              ? "No sources selected. Agents will see no tools."
              : active.map((source) => source.name).join(", ")}
          </p>
          <p className="mt-2 text-sm text-muted-foreground">
            Tools in these sources:{" "}
            {active.reduce((n, source) => n + source.toolCount, 0)}
          </p>
        </section>
        <ErrorNotice error={save.error} />
        {!workspace?.allSources && (
          <Button type="submit" disabled={save.isPending}>
            {save.isPending
              ? "Saving…"
              : workspace
                ? "Save workspace"
                : "Create workspace"}
          </Button>
        )}
      </FieldGroup>
    </form>
  )
}
