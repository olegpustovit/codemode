import { useState } from "react"
import { getRouteApi } from "@tanstack/react-router"
import { useMutation, useSuspenseQuery } from "@tanstack/react-query"
import { redirectResponse, write } from "@/lib/api"
import { consentQuery } from "./authorization-query"
import { workspacesQuery, sourcesQuery } from "./queries"
import { PageHeader, ErrorNotice } from "@/components/common"
import { Checkbox } from "@/components/ui/checkbox"
import { Button } from "@/components/ui/button"
import { Field, FieldLabel, FieldDescription } from "@/components/ui/field"
import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select"
import { Alert, AlertTitle, AlertDescription } from "@/components/ui/alert"
const route = getRouteApi("/_authenticated/authorize")
export function AuthorizePage() {
  const request = route.useSearch()
  const consent = useSuspenseQuery(consentQuery(request)).data
  const workspaces = useSuspenseQuery(workspacesQuery).data
  const sources = useSuspenseQuery(sourcesQuery).data
  const [workspaceId, setWorkspaceId] = useState(
    consent.requestedWorkspaceId ?? ""
  )
  const [admin, setAdmin] = useState(false)
  const workspace = workspaces.find((w) => w.id === workspaceId)
  const scope = workspace?.allSources
    ? sources
    : sources.filter((s) => workspace?.sourceIds.includes(s.id))
  const decision = useMutation({
    mutationFn: (allow: boolean) =>
      write(
        "/api/oauth/authorize",
        "POST",
        {
          ...request,
          ...(workspaceId ? { workspaceId } : {}),
          admin,
          allow,
        },
        redirectResponse
      ),
    onSuccess: (response) => window.location.assign(response.redirectUri),
  })
  return (
    <section className="mx-auto flex w-full max-w-2xl flex-col gap-6">
      <PageHeader
        title={`Authorize ${consent.clientName}?`}
        description="Choose which workspace this client can use."
      />
      <dl className="grid gap-2 text-sm break-all">
        <dt className="font-medium">Client identity</dt>
        <dd className="font-mono text-muted-foreground">{consent.clientId}</dd>
        <dt className="mt-3 font-medium">MCP URL</dt>
        <dd className="font-mono">{consent.resource}</dd>
        <dt className="mt-3 font-medium">Return address</dt>
        <dd className="font-mono text-muted-foreground">
          {consent.redirectUri}
        </dd>
      </dl>
      <Field>
        <FieldLabel htmlFor="oauth-workspace">Workspace</FieldLabel>
        <NativeSelect
          id="oauth-workspace"
          value={workspaceId}
          onChange={(event) => setWorkspaceId(event.target.value)}
        >
          <NativeSelectOption value="" disabled>
            Choose a workspace
          </NativeSelectOption>
          {workspaces.map((w) => (
            <NativeSelectOption key={w.id} value={w.id}>
              {w.name}
              {w.id === consent.requestedWorkspaceId
                ? " · from the MCP URL"
                : ""}
            </NativeSelectOption>
          ))}
        </NativeSelect>
        <FieldDescription>
          {consent.workspaceNotice ??
            "This authorization stays bound to your chosen workspace."}
        </FieldDescription>
      </Field>
      <Alert>
        <AlertTitle>
          {workspace
            ? `Access to ${workspace.name}`
            : "Choose a workspace to see its sources"}
        </AlertTitle>
        <AlertDescription>
          {workspace && (
            <>
              <p>
                {scope.length
                  ? scope.map((s) => s.slug).join(", ")
                  : "This workspace currently includes no sources."}
              </p>
              <p>
                The client can search and call tools in this workspace using
                your linked accounts, subject to its tool policies. Future
                changes to workspace membership also apply.
              </p>
            </>
          )}
        </AlertDescription>
      </Alert>
      <Field>
        <FieldLabel
          htmlFor="authorize-admin"
          className="flex items-center gap-2"
        >
          <Checkbox
            id="authorize-admin"
            checked={admin}
            onCheckedChange={setAdmin}
          />
          Also allow administration tools
        </FieldLabel>
        <FieldDescription>
          Manage workspaces and sources, synchronize tools, and see linked
          account metadata.
        </FieldDescription>
      </Field>
      <ErrorNotice error={decision.error} />
      <div className="flex flex-wrap justify-end gap-3">
        <Button
          variant="outline"
          disabled={decision.isPending}
          onClick={() => decision.mutate(false)}
        >
          Deny
        </Button>
        <Button
          disabled={decision.isPending || !workspace}
          onClick={() => decision.mutate(true)}
        >
          {decision.isPending
            ? "Returning to client…"
            : `Authorize ${workspace?.name ?? "workspace"}${admin ? " with administration" : ""}`}
        </Button>
      </div>
    </section>
  )
}
