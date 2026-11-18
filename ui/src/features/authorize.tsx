import { useState } from "react"
import { getRouteApi } from "@tanstack/react-router"
import { useMutation, useSuspenseQuery } from "@tanstack/react-query"
import { redirectResponse, write } from "@/lib/api"
import { consentQuery } from "./authorization-query"
import { workspacesQuery, sourcesQuery } from "./queries"
import { Facts } from "@/components/facts"
import { ErrorNotice } from "@/components/common"
import { Checkbox } from "@/components/ui/checkbox"
import { Button } from "@/components/ui/button"
import { Field, FieldLabel, FieldDescription } from "@/components/ui/field"
import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select"
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
    <section className="consent">
      <header>
        <h1>Authorize {consent.clientName}?</h1>
        <p className="consent-lead">
          {consent.requestedWorkspaceId ? (
            <>
              {consent.clientName} asked for{" "}
              <strong>
                {
                  workspaces.find(
                    (item) => item.id === consent.requestedWorkspaceId
                  )?.name
                }
              </strong>{" "}
              through its MCP URL. You can pick another workspace.
            </>
          ) : (
            "Choose which workspace this client can use."
          )}
        </p>
      </header>
      <Facts
        items={[
          { label: "Client", value: consent.clientId },
          { label: "MCP URL", value: consent.resource },
          { label: "Returns to", value: consent.redirectUri },
        ]}
      />
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
          {consent.workspaceNotice ?? "Tokens stay bound to this workspace."}
        </FieldDescription>
        {workspace && (
          <section aria-label="Workspace sources">
            <p className="consent-hint">Sources</p>
            {scope.length ? (
              <ul className="consent-sources">
                {scope.map((source) => (
                  <li key={source.id}>{source.slug}</li>
                ))}
              </ul>
            ) : (
              <p className="consent-hint">
                This workspace currently includes no sources.
              </p>
            )}
          </section>
        )}
      </Field>
      <div className="consent-admin">
        <Checkbox
          id="authorize-admin"
          checked={admin}
          onCheckedChange={setAdmin}
        />
        <label htmlFor="authorize-admin">Also allow administration tools</label>
        <p>
          Lets {consent.clientName} manage your workspaces and sources from
          code. Administration is off by default and every call is audited.
        </p>
        <ul>
          <li>
            codemode.admin.list_workspaces / save_workspace / delete_workspace
          </li>
          <li>
            codemode.admin.list_sources / save_source / delete_source /
            sync_source
          </li>
          <li>codemode.admin.list_accounts / source_connection_url</li>
        </ul>
      </div>
      <p className="consent-hint">
        Always included: <code>codemode.save_script</code> saves reusable Python
        {workspace ? ` to ${workspace.name}` : ""}.
      </p>
      <ErrorNotice error={decision.error} />
      <div className="consent-actions">
        <Button
          disabled={decision.isPending || !workspace}
          onClick={() => decision.mutate(true)}
        >
          {decision.isPending
            ? "Returning to client…"
            : `Authorize ${workspace?.name ?? "workspace"}${admin ? " with administration" : ""}`}
        </Button>
        <Button
          variant="ghost"
          disabled={decision.isPending}
          onClick={() => decision.mutate(false)}
        >
          Deny
        </Button>
      </div>
    </section>
  )
}
