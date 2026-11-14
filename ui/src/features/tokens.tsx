import { useState } from "react"
import {
  useMutation,
  useQueryClient,
  useSuspenseQuery,
} from "@tanstack/react-query"
import { z } from "zod"
import { Copy } from "lucide-react"
import { tokensQuery, workspacesQuery } from "./queries"
import { write, nothing } from "@/lib/api"
import { PageHeader, Blank, ErrorNotice } from "@/components/common"
import { ConfirmAction } from "@/components/confirm-action"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import {
  Field,
  FieldGroup,
  FieldLabel,
  FieldDescription,
} from "@/components/ui/field"
import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select"
import {
  Table,
  TableHeader,
  TableHead,
  TableBody,
  TableRow,
  TableCell,
} from "@/components/ui/table"
import { Alert, AlertTitle, AlertDescription } from "@/components/ui/alert"
import { toast } from "@/components/ui/toast"
export function TokensPage() {
  const [administration, setAdministration] = useState(false)
  const tokens = useSuspenseQuery(tokensQuery).data
  const workspaces = useSuspenseQuery(workspacesQuery).data
  const cache = useQueryClient()
  const create = useMutation({
    mutationFn: (form: FormData) =>
      write(
        "/api/tokens",
        "POST",
        {
          name: z.string().trim().min(1).max(128).parse(form.get("name")),
          administration,
          ...(administration
            ? {}
            : { workspaceId: z.uuid().parse(form.get("workspace")) }),
        },
        z.object({ id: z.uuid(), token: z.string().min(1) })
      ),
    onSuccess: () => cache.invalidateQueries({ queryKey: ["tokens"] }),
  })
  return (
    <>
      <PageHeader
        title="Access tokens"
        description="Use a personal token for a headless agent, or connect through OAuth."
      />
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
                    .then(() =>
                      toast.add({ title: "Token copied", type: "success" })
                    )
                    .catch(() =>
                      toast.add({
                        title: "Copy failed. Select and copy the token.",
                        type: "error",
                      })
                    )
                }}
              >
                <Copy data-icon="inline-start" />
                Copy token
              </Button>
              <Button variant="outline" onClick={() => create.reset()}>
                I saved it
              </Button>
            </div>
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
              <FieldLabel htmlFor="token-access">Access</FieldLabel>
              <NativeSelect
                id="token-access"
                value={administration ? "admin" : "mcp"}
                onChange={(event) =>
                  setAdministration(event.target.value === "admin")
                }
              >
                <NativeSelectOption value="mcp">
                  Workspace tools · /mcp
                </NativeSelectOption>
                <NativeSelectOption value="admin">
                  Administration · /mcp-admin
                </NativeSelectOption>
              </NativeSelect>
              <FieldDescription>
                {administration
                  ? "Can create, change and delete your workspaces and sources. Credentials stay private."
                  : "Can search and call tools in one workspace."}
              </FieldDescription>
            </Field>
            {!administration && (
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
            )}
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
      <section className="flex flex-col gap-4">
        <h2>Issued tokens</h2>
        {tokens.length === 0 ? (
          <Blank
            title="No tokens issued"
            description="Create a token above, or authorize an OAuth client."
          />
        ) : (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Name</TableHead>
                <TableHead>Access</TableHead>
                <TableHead>Status</TableHead>
                <TableHead>
                  <span className="sr-only">Revoke</span>
                </TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {tokens.map((token) => (
                <TableRow key={token.id}>
                  <TableCell className="max-w-xs break-all">
                    {token.name}
                  </TableCell>
                  <TableCell className="font-mono text-xs break-all">
                    {token.resource === "/mcp-admin"
                      ? "Administration"
                      : (workspaces.find((w) => w.id === token.workspaceId)
                          ?.name ?? "Deleted workspace")}
                    <p className="mt-1 text-muted-foreground">
                      {token.resource}
                    </p>
                  </TableCell>
                  <TableCell>{token.revoked ? "Revoked" : "Issued"}</TableCell>
                  <TableCell>
                    {!token.revoked && (
                      <ConfirmAction
                        label="Revoke token"
                        description="The agent will lose access. OAuth token revocation also revokes its refresh grant."
                        onConfirm={async () => {
                          await write(
                            `/api/tokens/${token.id}`,
                            "DELETE",
                            undefined,
                            nothing
                          )
                          await cache.invalidateQueries({
                            queryKey: ["tokens"],
                          })
                        }}
                      />
                    )}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        )}
      </section>
    </>
  )
}
