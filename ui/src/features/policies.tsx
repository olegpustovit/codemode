import { useState } from "react"
import { Link, getRouteApi, useRouter } from "@tanstack/react-router"
import {
  useMutation,
  useQueryClient,
  useSuspenseQuery,
} from "@tanstack/react-query"
import { ArrowLeft } from "lucide-react"
import type { Workspace, ToolPolicy } from "./queries"
import { workspaceQuery, policyQuery } from "./queries"
import { nothing, write } from "@/lib/api"
import { PageHeader, Blank, ErrorNotice, StateBadge } from "@/components/common"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Checkbox } from "@/components/ui/checkbox"
import { Field, FieldLabel } from "@/components/ui/field"
import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select"
import {
  Table,
  TableHeader,
  TableHead,
  TableBody,
  TableRow,
  TableCell,
} from "@/components/ui/table"
const route = getRouteApi("/_authenticated/workspaces/$workspaceId_/policies")
export function PoliciesPage() {
  const { workspaceId } = route.useParams()
  const { q, cursor } = route.useSearch()
  const workspace = useSuspenseQuery(workspaceQuery(workspaceId)).data
  const policies = useSuspenseQuery(policyQuery(workspaceId, q, cursor)).data
  const router = useRouter()
  return (
    <>
      <Link
        to="/workspaces/$workspaceId"
        params={{ workspaceId }}
        className="flex items-center gap-2 text-sm text-muted-foreground"
      >
        <ArrowLeft className="size-4" />
        {workspace.name}
      </Link>
      <PageHeader
        title="Tool policies"
        description={`Choose which calls are allowed at ${workspace.name}. Tools default to allow; blocked calls raise PermissionError.`}
      />
      <ToolScopeMode
        workspace={workspace}
        key={`${workspace.id}-${workspace.allTools}`}
      />
      <form
        className="flex flex-wrap items-end gap-3"
        key={`${workspaceId}-${q}`}
        onSubmit={(event) => {
          event.preventDefault()
          const query = String(new FormData(event.currentTarget).get("q") ?? "")
          void router.navigate({
            to: "/workspaces/$workspaceId/policies",
            params: { workspaceId },
            search: { q: query || undefined },
          })
        }}
      >
        <Field className="max-w-md">
          <FieldLabel htmlFor="policy-search">Find a tool</FieldLabel>
          <Input
            id="policy-search"
            name="q"
            defaultValue={q}
            maxLength={1024}
            placeholder="Search tool names and descriptions"
          />
        </Field>
        <Button type="submit" variant="outline">
          Search
        </Button>
      </form>
      {!policies.tools.length ? (
        <Blank
          title="No matching tools"
          description="Change your search or connect a source included in this workspace."
        />
      ) : (
        <Table className="block sm:table">
          <TableHeader className="hidden sm:table-header-group">
            <TableRow>
              <TableHead>Tool</TableHead>
              <TableHead>Included</TableHead>
              <TableHead>Effective policy</TableHead>
              <TableHead>Policy override</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody className="block sm:table-row-group">
            {policies.tools.map((tool) => (
              <PolicyRow
                key={`${workspaceId}-${tool.sourceId}-${tool.upstreamName}-${tool.sourceRevision}-${tool.overrideAction}-${tool.inScope}`}
                workspaceId={workspaceId}
                allTools={workspace.allTools}
                tool={tool}
              />
            ))}
          </TableBody>
        </Table>
      )}
      <div className="flex flex-wrap justify-between gap-3">
        <Button
          variant="outline"
          disabled={!cursor}
          render={
            <Link
              to="/workspaces/$workspaceId/policies"
              params={{ workspaceId }}
              search={{ q }}
            />
          }
        >
          First page
        </Button>
        {policies.nextCursor && (
          <Button
            variant="outline"
            render={
              <Link
                to="/workspaces/$workspaceId/policies"
                params={{ workspaceId }}
                search={{ q, cursor: policies.nextCursor }}
              />
            }
          >
            Next page
          </Button>
        )}
      </div>
    </>
  )
}
function PolicyRow({
  workspaceId,
  allTools,
  tool,
}: {
  workspaceId: string
  allTools: boolean
  tool: ToolPolicy
}) {
  const [choice, setChoice] = useState(tool.overrideAction ?? "default")
  const [included, setIncluded] = useState(tool.inScope)
  const cache = useQueryClient()
  const save = useMutation({
    mutationFn: () =>
      write(
        `/api/workspaces/${workspaceId}/policies`,
        "PUT",
        {
          sourceId: tool.sourceId,
          sourceRevision: tool.sourceRevision,
          upstreamName: tool.upstreamName,
          action: choice,
        },
        nothing
      ),
    onSuccess: async () => {
      await cache.invalidateQueries({ queryKey: ["policies", workspaceId] })
    },
  })
  const scope = useMutation({
    onError: () => setIncluded(tool.inScope),
    mutationFn: (included: boolean) =>
      write(
        `/api/workspaces/${workspaceId}/scope/tools`,
        "PUT",
        {
          sourceId: tool.sourceId,
          sourceRevision: tool.sourceRevision,
          upstreamName: tool.upstreamName,
          included,
        },
        nothing
      ),
    onSuccess: async () => {
      await cache.invalidateQueries({ queryKey: ["policies", workspaceId] })
    },
  })
  return (
    <TableRow className="grid grid-cols-2 gap-x-4 gap-y-3 py-5 sm:table-row sm:py-0">
      <TableCell className="col-span-2 max-w-md p-0 whitespace-normal sm:p-2">
        <p className="font-mono text-xs break-all">{tool.path}</p>
        <p className="mt-1 line-clamp-2 text-muted-foreground">
          {tool.description}
        </p>
      </TableCell>
      <TableCell className="flex items-center gap-2 p-0 sm:table-cell sm:p-2">
        <span className="text-muted-foreground sm:hidden">Included</span>
        <Checkbox
          aria-label={`Include ${tool.path}`}
          checked={included}
          disabled={allTools || scope.isPending || save.isPending}
          onCheckedChange={(value) => {
            setIncluded(value === true)
            scope.mutate(value === true)
          }}
        />
        <ErrorNotice error={scope.error} />
      </TableCell>
      <TableCell className="p-0 sm:p-2">
        <StateBadge
          state={!tool.inScope ? "outside_scope" : tool.effectiveAction}
        />
      </TableCell>
      <TableCell className="col-span-2 p-0 sm:p-2">
        <form
          className="flex flex-wrap items-center gap-2"
          onSubmit={(event) => {
            event.preventDefault()
            save.mutate()
          }}
        >
          <NativeSelect
            aria-label={`Policy for ${tool.path}`}
            value={choice}
            onChange={(event) => setChoice(event.target.value)}
            disabled={save.isPending || scope.isPending}
          >
            <NativeSelectOption value="default">Use default</NativeSelectOption>
            <NativeSelectOption value="allow">Allow</NativeSelectOption>
            <NativeSelectOption value="block">Block</NativeSelectOption>
          </NativeSelect>
          <Button
            type="submit"
            variant="outline"
            disabled={
              save.isPending ||
              scope.isPending ||
              choice === (tool.overrideAction ?? "default")
            }
            aria-label={`Save policy for ${tool.path}`}
          >
            {save.isPending ? "Saving…" : "Save"}
          </Button>
          <ErrorNotice error={save.error} />
        </form>
      </TableCell>
    </TableRow>
  )
}

function ToolScopeMode({ workspace }: { workspace: Workspace }) {
  const [mode, setMode] = useState(workspace.allTools ? "all" : "selected")
  const cache = useQueryClient()
  const save = useMutation({
    mutationFn: () =>
      write(
        `/api/workspaces/${workspace.id}/scope`,
        "PUT",
        { allTools: mode === "all" },
        nothing
      ),
    onSuccess: async () => {
      await Promise.all([
        cache.invalidateQueries({ queryKey: ["workspaces"] }),
        cache.invalidateQueries({ queryKey: ["policies", workspace.id] }),
      ])
    },
  })
  return (
    <form
      className="flex flex-col gap-3 border-b pb-5"
      onSubmit={(event) => {
        event.preventDefault()
        save.mutate()
      }}
    >
      <Field className="max-w-md">
        <FieldLabel htmlFor="tool-scope-mode">Tool scope</FieldLabel>
        <NativeSelect
          id="tool-scope-mode"
          value={mode}
          onChange={(event) => setMode(event.target.value)}
          disabled={save.isPending}
        >
          <NativeSelectOption value="all">
            All tools from included sources
          </NativeSelectOption>
          <NativeSelectOption value="selected">
            Only selected tools
          </NativeSelectOption>
        </NativeSelect>
      </Field>
      <p className="max-w-prose text-sm text-muted-foreground">
        Selected scope hides every tool until you include it below. New tools
        stay hidden. Policies can restrict an included tool further.
      </p>
      <ErrorNotice error={save.error} />
      <Button
        className="self-start"
        variant="outline"
        type="submit"
        disabled={
          save.isPending || mode === (workspace.allTools ? "all" : "selected")
        }
      >
        {save.isPending ? "Saving…" : "Save tool scope"}
      </Button>
    </form>
  )
}
