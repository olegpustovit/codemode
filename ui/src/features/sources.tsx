import { useState } from "react"
import { Link, getRouteApi, useRouter } from "@tanstack/react-router"
import {
  useMutation,
  useQueryClient,
  useSuspenseQuery,
} from "@tanstack/react-query"
import { RefreshCw, ArrowLeft } from "lucide-react"
import { z } from "zod"
import { sourceQuery, workspacesQuery, integrationsQuery } from "./queries"
import type { Source } from "./queries"
import { created, nothing, write } from "@/lib/api"
import { PageHeader, StateBadge, ErrorNotice, date } from "@/components/common"
import { ConfirmAction } from "@/components/confirm-action"
import { Button } from "@/components/ui/button"
import { IconPicker } from "@/components/icon-picker"
import { Input } from "@/components/ui/input"
import { Textarea } from "@/components/ui/textarea"
import {
  Field,
  FieldGroup,
  FieldLabel,
  FieldDescription,
} from "@/components/ui/field"
import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select"
import { Checkbox } from "@/components/ui/checkbox"

import { Alert, AlertTitle, AlertDescription } from "@/components/ui/alert"
const detailRoute = getRouteApi("/_authenticated/sources/$sourceId")
export function NewSourcePage() {
  return (
    <>
      <PageHeader
        title="Add source"
        description="Choose the MCP server, OpenAPI document or GraphQL endpoint your agents will use."
      />
      <SourceEditor />
    </>
  )
}
export function SourcePage() {
  const { sourceId } = detailRoute.useParams()
  const source = useSuspenseQuery(sourceQuery(sourceId)).data
  const workspaces = useSuspenseQuery(workspacesQuery).data
  const cache = useQueryClient()
  const router = useRouter()
  const sync = useMutation({
    mutationFn: () =>
      write(`/api/sources/${sourceId}/sync`, "POST", undefined, nothing),
    onSuccess: () =>
      Promise.all([
        cache.invalidateQueries({ queryKey: ["sources"] }),
        cache.invalidateQueries({ queryKey: ["integrations"] }),
      ]),
  })
  return (
    <>
      <Link
        to="/sources"
        className="flex items-center gap-2 text-sm text-muted-foreground"
      >
        <ArrowLeft className="size-4" />
        Sources
      </Link>
      <PageHeader
        title={source.name}
        description={`Python namespace: tools.${source.pythonName}`}
        action={
          <div className="flex gap-2">
            <Button
              variant="outline"
              render={
                <Link to="/sources/$sourceId/accounts" params={{ sourceId }} />
              }
            >
              Manage account
            </Button>
            <Button
              variant="outline"
              onClick={() => sync.mutate()}
              disabled={sync.isPending}
            >
              <RefreshCw data-icon="inline-start" />
              {sync.isPending ? "Requesting…" : "Sync now"}
            </Button>
          </div>
        }
      />
      <ErrorNotice error={sync.error} />
      {source.lastError && (
        <Alert variant="destructive">
          <AlertTitle>Source needs attention</AlertTitle>
          <AlertDescription>
            {source.lastError} Check the server address and linked account, then
            sync again.
          </AlertDescription>
        </Alert>
      )}
      <div className="record-grid">
        <SourceEditor source={source} key={`${source.id}:${source.revision}`} />
        <aside className="flex flex-col gap-6">
          <section className="flex flex-col gap-3">
            <h2>Catalogue</h2>
            <StateBadge state={source.status} />
            <p>
              Tools: {source.toolCount} · Last sync {date(source.syncedAt)}
            </p>
          </section>
          <section className="flex flex-col gap-3">
            <h2>Available through</h2>
            {workspaces
              .filter(
                (workspace) =>
                  workspace.allSources || workspace.sourceIds.includes(sourceId)
              )
              .map((workspace) => (
                <Link
                  to="/workspaces/$workspaceId"
                  params={{ workspaceId: workspace.id }}
                  className="text-sm break-all underline underline-offset-4"
                  key={workspace.id}
                >
                  {workspace.name}
                </Link>
              ))}
          </section>
          <section className="flex flex-col gap-3">
            <h2>Remove source</h2>
            <p className="text-sm text-muted-foreground">
              Remove its tools and linked account from this gateway.
            </p>
            <ConfirmAction
              label="Remove source"
              description="The source, catalogue entries and linked credentials will be removed. Existing execution logs remain."
              onConfirm={async () => {
                await write(
                  `/api/sources/${sourceId}`,
                  "DELETE",
                  undefined,
                  nothing
                )
                await cache.invalidateQueries({ queryKey: ["sources"] })
                await cache.invalidateQueries({ queryKey: ["accounts"] })
                await cache.invalidateQueries({ queryKey: ["workspaces"] })
                await cache.invalidateQueries({ queryKey: ["policies"] })
                await router.navigate({ to: "/sources" })
              }}
            />
          </section>
        </aside>
      </div>
    </>
  )
}
function SourceEditor({ source }: { source?: Source }) {
  const integrations = useSuspenseQuery(integrationsQuery).data
  const preset = integrations.some(
    (entry) =>
      entry.sourceId === source?.id && !entry.slug.startsWith("source-")
  )
  const [kind, setKind] = useState(source?.kind ?? "mcp_http")
  const [displayName, setDisplayName] = useState(source?.name ?? "")
  const cache = useQueryClient()
  const router = useRouter()
  const save = useMutation({
    mutationFn: async (form: FormData) => {
      const name = z.string().min(1).max(120).parse(form.get("name"))
      const slug =
        source?.slug ??
        (name
          .toLowerCase()
          .replace(/[^a-z0-9_-]+/g, "_")
          .replace(/^_+|_+$/g, "")
          .slice(0, 64) ||
          "source")
      const icon = String(form.get("icon") ?? source?.icon ?? "plug")
      if (source?.kind === "mcp_stdio") {
        await write(
          `/api/sources/${source.id}`,
          "PATCH",
          { name, icon },
          nothing
        )
        return source.id
      }
      const requiresAccount = form.get("requiresAccount") === "on"
      const transport =
        kind !== "mcp_stdio"
          ? {
              kind,
              url: z.url().parse(form.get("url")),
              ...(kind === "openapi"
                ? {
                    serverUrl: form.get("serverUrl")
                      ? z.url().parse(form.get("serverUrl"))
                      : undefined,
                  }
                : {}),
            }
          : {
              kind,
              command: z.string().startsWith("/").parse(form.get("command")),
              args: z
                .array(z.string())
                .parse(JSON.parse(String(form.get("args") || "[]"))),
            }
      const body = {
        slug,
        ...(!preset ? { name, icon } : {}),
        requiresAccount,
        ...transport,
      }
      if (source) {
        await write(`/api/sources/${source.id}`, "PUT", body, nothing)
        return source.id
      }
      return (await write("/api/sources", "POST", body, created)).id
    },
    onSuccess: async (id) => {
      await Promise.all([
        cache.invalidateQueries({ queryKey: ["sources"] }),
        cache.invalidateQueries({ queryKey: ["integrations"] }),
        cache.invalidateQueries({ queryKey: ["accounts"] }),
        cache.invalidateQueries({ queryKey: ["policies"] }),
      ])
      await router.navigate({
        to: "/sources/$sourceId",
        params: { sourceId: id },
      })
    },
  })
  return (
    <form
      className="form-section"
      onSubmit={(event) => {
        event.preventDefault()
        save.mutate(new FormData(event.currentTarget))
      }}
    >
      <FieldGroup>
        <Field>
          <FieldLabel htmlFor="source-name">Name</FieldLabel>
          <Input
            id="source-name"
            name="name"
            value={displayName}
            onChange={(event) => setDisplayName(event.target.value)}
            required
            maxLength={120}
            readOnly={preset}
            placeholder="Acme Billing"
          />
          <FieldDescription>
            {source ? (
              <>
                Python namespace: <code>tools.{source.pythonName}</code>.
                Renaming keeps it unchanged.
              </>
            ) : (
              "The Python namespace is derived once from this name."
            )}
          </FieldDescription>
        </Field>
        {!preset && (
          <IconPicker
            selected={source?.icon ?? "plug"}
            label={displayName || "Source"}
          />
        )}
        <Field>
          <FieldLabel htmlFor="kind">Connection type</FieldLabel>
          <NativeSelect
            id="kind"
            name="kind"
            disabled={source?.kind === "mcp_stdio"}
            value={kind}
            onChange={(e) =>
              setKind(
                z
                  .enum(["mcp_http", "mcp_stdio", "openapi", "graphql"])
                  .parse(e.target.value)
              )
            }
          >
            <NativeSelectOption value="mcp_http">
              Remote MCP over HTTP
            </NativeSelectOption>
            <NativeSelectOption value="openapi">
              OpenAPI document
            </NativeSelectOption>
            <NativeSelectOption value="graphql">
              GraphQL endpoint
            </NativeSelectOption>
            {source?.kind === "mcp_stdio" && (
              <NativeSelectOption value="mcp_stdio">
                Local stdio server (managed by the server operator)
              </NativeSelectOption>
            )}
          </NativeSelect>
        </Field>
        {kind !== "mcp_stdio" ? (
          <>
            <Field>
              <FieldLabel htmlFor="source-url">
                {kind === "openapi"
                  ? "OpenAPI document address"
                  : kind === "graphql"
                    ? "GraphQL endpoint address"
                    : "Server address"}
              </FieldLabel>
              <Input
                id="source-url"
                name="url"
                type="url"
                required
                defaultValue={
                  source && source.kind !== "mcp_stdio" ? source.url : ""
                }
                placeholder={
                  kind === "graphql"
                    ? "https://provider.example/graphql"
                    : "https://provider.example/mcp"
                }
              />
              <FieldDescription>
                {kind === "openapi"
                  ? "Use an OpenAPI 3.x JSON or YAML document. Local references are supported."
                  : kind === "graphql"
                    ? "Exposes query and mutation tools. Tools default to allow and can be blocked per workspace. Link credentials on the account page."
                    : "Use the Streamable HTTP endpoint. Enter credentials on the account page."}
              </FieldDescription>
            </Field>
            {kind === "openapi" && (
              <Field>
                <FieldLabel htmlFor="server-url">API server address</FieldLabel>
                <Input
                  id="server-url"
                  name="serverUrl"
                  type="url"
                  defaultValue={
                    source?.kind === "openapi" ? source.serverUrl : ""
                  }
                  placeholder="Read from the document when blank"
                />
                <FieldDescription>
                  The server address is saved when you add the source. Later
                  document changes cannot move requests to another server.
                  Changing this address removes the linked credential.
                </FieldDescription>
              </Field>
            )}
          </>
        ) : (
          <>
            <Field>
              <FieldLabel htmlFor="command">Executable</FieldLabel>
              <Input
                id="command"
                name="command"
                readOnly
                required
                defaultValue={
                  source?.kind === "mcp_stdio" ? source.command : ""
                }
                placeholder="/usr/local/bin/mcp-server"
              />
              <FieldDescription>
                Operator-managed connection. You can change its name and icon.
              </FieldDescription>
            </Field>
            <Field>
              <FieldLabel htmlFor="args">Arguments (JSON array)</FieldLabel>
              <Textarea
                id="args"
                name="args"
                readOnly
                className="font-mono"
                defaultValue={JSON.stringify(
                  source?.kind === "mcp_stdio" ? source.args : [],
                  null,
                  2
                )}
              />
            </Field>
          </>
        )}
        <Field orientation="horizontal">
          <Checkbox
            id="required-account"
            name="requiresAccount"
            defaultChecked={source?.requiresAccount}
          />
          <FieldLabel htmlFor="required-account">
            Require a linked account
          </FieldLabel>
        </Field>
        {source && (
          <p className="text-sm text-muted-foreground">
            Changing the server address, executable or arguments removes the
            existing linked account.
          </p>
        )}
        <ErrorNotice error={save.error} />
        <Button type="submit" disabled={save.isPending}>
          {save.isPending ? "Saving…" : source ? "Save changes" : "Add source"}
        </Button>
      </FieldGroup>
    </form>
  )
}
