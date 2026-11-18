import { useState } from "react"
import {
  useMutation,
  useQueryClient,
  useSuspenseQuery,
} from "@tanstack/react-query"
import { z } from "zod"
import { Copy } from "lucide-react"
import { workspacesQuery } from "./queries"
import { mcpUrl } from "@/lib/workspace-url"
import { configQuery } from "@/lib/runtime"
import { write } from "@/lib/api"
import { ErrorNotice } from "@/components/common"
import { Checkbox } from "@/components/ui/checkbox"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import {
  Field,
  FieldGroup,
  FieldLabel,
  FieldDescription,
} from "@/components/ui/field"
import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select"
import { Alert, AlertTitle, AlertDescription } from "@/components/ui/alert"
import { useStatus } from "@/components/status-bar"
export function TokenForm({ onDone }: { onDone: () => void }) {
  const { report } = useStatus()
  const [admin, setAdmin] = useState(false)
  const workspaces = useSuspenseQuery(workspacesQuery).data
  const cache = useQueryClient()
  const serverName = "codemode"
  const { hostname } = useSuspenseQuery(configQuery).data
  const create = useMutation({
    mutationFn: async (form: FormData) => {
      const workspaceId = z.uuid().parse(form.get("workspace"))
      const result = await write(
        "/api/tokens",
        "POST",
        {
          name: z.string().trim().min(1).max(128).parse(form.get("name")),
          admin,
          workspaceId,
        },
        z.object({ id: z.uuid(), token: z.string().min(1) })
      )
      return {
        ...result,
        url: mcpUrl(hostname, workspaceId),
      }
    },
    onSuccess: () => cache.invalidateQueries({ queryKey: ["tokens"] }),
  })
  return (
    <>
      {create.data ? (
        <Alert>
          <AlertTitle>Save this token now</AlertTitle>
          <AlertDescription>
            <p>
              It will not be shown again. Store it in your agent's secret
              configuration.
            </p>
            <Field className="mt-3">
              <FieldLabel htmlFor="new-token">New access token</FieldLabel>
              <Input
                id="new-token"
                readOnly
                value={create.data.token}
                className="font-mono"
                autoComplete="off"
              />
            </Field>
            <div className="mt-3 flex gap-3">
              <Button
                variant="outline"
                onClick={() => {
                  void navigator.clipboard
                    .writeText(create.data.token)
                    .then(() => report("Token copied"))
                    .catch(() =>
                      report("Copy failed. Select and copy the token.", "error")
                    )
                }}
              >
                <Copy data-icon="inline-start" />
                Copy token
              </Button>
              <Button variant="outline" onClick={onDone}>
                I saved it
              </Button>
            </div>
            <p>
              Set CODEMODE_TOKEN in your agent's secret environment to the token
              above, then connect Codex:
            </p>
            <pre className="code-panel">
              <code>{`codex mcp add ${serverName} --url ${create.data.url} --bearer-token-env-var CODEMODE_TOKEN`}</code>
            </pre>
            <p>
              In the existing <code>{`[mcp_servers.${serverName}]`}</code>{" "}
              section of <code>~/.codex/config.toml</code>, add:
            </p>
            <pre className="code-panel">
              <code>{`default_tools_approval_mode = "approve"\nomit_tools_from = ["deferred"]`}</code>
            </pre>
            <p>
              For Codex 0.169.0 sessions using only this gateway, merge these
              values into the existing <code>[features]</code> section:
            </p>
            <pre className="code-panel">
              <code>
                {
                  "code_mode = true\ntool_suggest = true\ncode_mode_tool_search = false"
                }
              </code>
            </pre>
            <p>
              If ChatGPT connectors cover the same apps, set{" "}
              <code>apps = false</code> in the existing <code>[features]</code>{" "}
              section, or disconnect duplicate connectors.
            </p>
          </AlertDescription>
        </Alert>
      ) : (
        <form
          className="form-section section-rule"
          onSubmit={(event) => {
            event.preventDefault()
            create.mutate(new FormData(event.currentTarget))
          }}
        >
          <h2>Create personal token</h2>
          <FieldGroup>
            <Field>
              <FieldLabel htmlFor="token-name">Name</FieldLabel>
              <Input
                id="token-name"
                name="name"
                required
                maxLength={128}
                placeholder="Laptop agent"
              />
            </Field>
            <Field>
              <FieldLabel htmlFor="token-workspace">Workspace</FieldLabel>
              <NativeSelect
                id="token-workspace"
                name="workspace"
                defaultValue={workspaces[0]?.id}
              >
                {workspaces.map((workspace) => (
                  <NativeSelectOption key={workspace.id} value={workspace.id}>
                    {workspace.name}
                    {workspace.allSources ? " · all sources" : ""}
                  </NativeSelectOption>
                ))}
              </NativeSelect>
              <FieldDescription>
                The token stays bound to this workspace.
              </FieldDescription>
            </Field>
            <Field>
              <FieldLabel
                htmlFor="token-admin"
                className="flex items-center gap-2"
              >
                <Checkbox
                  id="token-admin"
                  checked={admin}
                  onCheckedChange={setAdmin}
                />
                Also allow administration tools
              </FieldLabel>
              <FieldDescription>
                Manage workspaces and sources and see linked account metadata.
              </FieldDescription>
            </Field>
            <ErrorNotice error={create.error} />
            <Button
              type="submit"
              disabled={create.isPending || workspaces.length === 0}
            >
              {create.isPending ? "Creating…" : "Create token"}
            </Button>
          </FieldGroup>
        </form>
      )}
    </>
  )
}
