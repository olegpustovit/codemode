import { useState } from "react"
import { Link, useRouter } from "@tanstack/react-router"
import {
  useMutation,
  useQueryClient,
  useSuspenseQuery,
} from "@tanstack/react-query"
import { ArrowLeft, Plus } from "lucide-react"
import {
  workspaceQuery,
  scriptsQuery,
  scriptQuery,
  scriptSchema,
} from "./queries"
import type { Script } from "./queries"
import { write, nothing } from "@/lib/api"
import { Blank, PageHeader, ErrorNotice, date } from "@/components/common"
import { ConfirmAction } from "@/components/confirm-action"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Textarea } from "@/components/ui/textarea"
import {
  Field,
  FieldLabel,
  FieldDescription,
  FieldGroup,
} from "@/components/ui/field"

export function ScriptsPage({ workspaceId }: { workspaceId: string }) {
  const workspace = useSuspenseQuery(workspaceQuery(workspaceId)).data
  const scripts = useSuspenseQuery(scriptsQuery(workspaceId)).data
  const create = (
    <Button
      render={
        <Link
          to="/workspaces/$workspaceId/scripts/new"
          params={{ workspaceId }}
        />
      }
    >
      <Plus data-icon="inline-start" />
      Create script
    </Button>
  )
  return (
    <>
      <Back workspaceId={workspaceId} />
      <PageHeader
        title="Saved scripts"
        description={`Reusable Python for ${workspace.name}. Each run uses this workspace’s current tool policies.`}
        action={scripts.length ? create : undefined}
      />
      {scripts.length ? (
        <div className="divide-y border-y">
          {scripts.map((script) => (
            <article
              key={script.id}
              className="flex flex-wrap items-start justify-between gap-3 py-5"
            >
              <div className="min-w-0 flex-1 basis-64">
                <Link
                  to="/workspaces/$workspaceId/scripts/$scriptId"
                  params={{ workspaceId, scriptId: script.id }}
                  className="font-medium break-words underline underline-offset-4"
                >
                  {script.name}
                </Link>
                <p className="mt-2 max-w-prose text-sm break-words whitespace-pre-wrap text-muted-foreground">
                  {script.description || "No description"}
                </p>
              </div>
              <p className="text-sm text-muted-foreground">
                Updated {date(script.updatedAt)}
              </p>
            </article>
          ))}
        </div>
      ) : (
        <Blank
          title="No saved scripts"
          description="Save Python you want agents to find and run again. Scripts are available only at this workspace."
        >
          {create}
        </Blank>
      )}
    </>
  )
}
function Back({ workspaceId }: { workspaceId: string }) {
  return (
    <Link
      to="/workspaces/$workspaceId"
      params={{ workspaceId }}
      className="flex items-center gap-2 text-sm text-muted-foreground"
    >
      <ArrowLeft className="size-4" />
      Workspace
    </Link>
  )
}
export function NewScriptPage({ workspaceId }: { workspaceId: string }) {
  const workspace = useSuspenseQuery(workspaceQuery(workspaceId)).data
  return (
    <>
      <Back workspaceId={workspaceId} />
      <PageHeader
        title="Create script"
        description={`Save reusable Python for ${workspace.name}.`}
      />
      <ScriptEditor workspaceId={workspaceId} />
    </>
  )
}
export function ScriptPage({
  workspaceId,
  scriptId,
}: {
  workspaceId: string
  scriptId: string
}) {
  const script = useSuspenseQuery(scriptQuery(workspaceId, scriptId)).data
  return (
    <>
      <Back workspaceId={workspaceId} />
      <PageHeader
        title="Edit script"
        description="Saving changes affects future runs. Executions already awaiting approval keep their original code."
      />
      <ScriptEditor key={script.id} workspaceId={workspaceId} script={script} />
    </>
  )
}
function ScriptEditor({
  workspaceId,
  script,
}: {
  workspaceId: string
  script?: Script
}) {
  const [name, setName] = useState(script?.name ?? "")
  const [description, setDescription] = useState(script?.description ?? "")
  const [code, setCode] = useState(script?.code ?? "")
  // Keep the revision belonging to this draft; a background refetch must not authorize overwriting another edit.
  const [revision, setRevision] = useState(script?.revision)
  const [saved, setSaved] = useState(false)
  const cache = useQueryClient()
  const router = useRouter()
  const bytes = new TextEncoder().encode(code).length
  const save = useMutation({
    mutationFn: () => {
      if (bytes > 65536)
        throw new Error(
          "Python source exceeds 64 KiB. Shorten it before saving."
        )
      return write(
        script
          ? `/api/workspaces/${workspaceId}/scripts/${script.id}`
          : `/api/workspaces/${workspaceId}/scripts`,
        script ? "PUT" : "POST",
        { name, description, code, ...(script ? { revision } : {}) },
        scriptSchema
      )
    },
    onSuccess: async (result) => {
      setRevision(result.revision)
      setName(result.name)
      setSaved(true)
      cache.setQueryData(scriptQuery(workspaceId, result.id).queryKey, result)
      await cache.invalidateQueries({
        queryKey: ["scripts", workspaceId],
        exact: true,
      })
      if (!script)
        await router.navigate({
          to: "/workspaces/$workspaceId/scripts/$scriptId",
          params: { workspaceId, scriptId: result.id },
        })
    },
  })
  return (
    <div className="record-grid">
      <form
        onSubmit={(event) => {
          event.preventDefault()
          setSaved(false)
          save.mutate()
        }}
        onChange={() => setSaved(false)}
      >
        <FieldGroup>
          <fieldset
            disabled={save.isPending}
            className="flex min-w-0 flex-col gap-5"
          >
            <Field>
              <FieldLabel htmlFor="script-name">Name</FieldLabel>
              <Input
                id="script-name"
                required
                maxLength={120}
                value={name}
                onChange={(event) => setName(event.target.value)}
              />
            </Field>
            <Field>
              <FieldLabel htmlFor="script-description">Description</FieldLabel>
              <Textarea
                id="script-description"
                maxLength={2000}
                value={description}
                onChange={(event) => setDescription(event.target.value)}
              />
              <FieldDescription>
                Describe when an agent should use this script. Names and
                descriptions appear in search.
              </FieldDescription>
            </Field>
            <Field>
              <FieldLabel htmlFor="script-code">Python source</FieldLabel>
              <Textarea
                id="script-code"
                spellCheck={false}
                className="min-h-80 font-mono text-sm"
                value={code}
                onChange={(event) => setCode(event.target.value)}
                aria-describedby="script-budget"
              />
              <FieldDescription id="script-budget">
                {bytes.toLocaleString()} / 65,536 bytes. Use await
                tools.source.tool(...) and return the result.
              </FieldDescription>
            </Field>
          </fieldset>
          <ErrorNotice error={save.error} />
          <div className="flex flex-wrap items-center gap-4">
            <Button type="submit" disabled={save.isPending || bytes > 65536}>
              {save.isPending
                ? "Saving…"
                : script
                  ? "Save changes"
                  : "Create script"}
            </Button>
            <output className="text-sm text-muted-foreground">
              {saved ? "Changes saved." : ""}
            </output>
          </div>
        </FieldGroup>
      </form>
      <aside className="flex min-w-0 flex-col gap-6">
        <section className="flex flex-col gap-3">
          <h2>Run from an agent</h2>
          <p className="text-sm text-muted-foreground">
            Agents can find this script through search on the same workspace.
            Each tool call still follows its current scope and approval rules.
          </p>
          {script && (
            <code className="code-panel block overflow-x-auto text-sm">{`execute(scriptId="${script.id}")`}</code>
          )}
        </section>
        <Button
          variant="outline"
          render={
            <Link
              to="/workspaces/$workspaceId/scripts"
              params={{ workspaceId }}
            />
          }
        >
          All saved scripts
        </Button>
        {script && (
          <ConfirmAction
            label="Delete script"
            description="Agents will no longer find or run this saved script. Existing executions and their audit history are retained."
            onConfirm={async () => {
              await write(
                `/api/workspaces/${workspaceId}/scripts/${script.id}`,
                "DELETE",
                { revision },
                nothing
              )
              await cache.invalidateQueries({
                queryKey: ["scripts", workspaceId],
              })
              await router.navigate({
                to: "/workspaces/$workspaceId/scripts",
                params: { workspaceId },
              })
            }}
          />
        )}
      </aside>
    </div>
  )
}
