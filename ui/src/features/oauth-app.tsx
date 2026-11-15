import { useState } from "react"
import {
  useMutation,
  useQueryClient,
  useSuspenseQuery,
} from "@tanstack/react-query"
import { configQuery } from "@/lib/runtime"
import { message, nothing, write } from "@/lib/api"
import { useStatus } from "@/components/status-bar"
import { ConfirmAction } from "@/components/confirm-action"
import { Facts } from "@/components/facts"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import type { Integration } from "./queries"
export function OAuthAppSetup({
  entry,
  configured,
}: {
  entry: Integration
  configured: boolean
}) {
  const config = useSuspenseQuery(configQuery).data
  const cache = useQueryClient()
  const { report } = useStatus()
  const [editing, setEditing] = useState(false)
  const [clientId, setClientId] = useState("")
  const [clientSecret, setClientSecret] = useState("")
  const refresh = () =>
    Promise.all(
      ["oauth-apps", "integrations", "sources"].map((key) =>
        cache.invalidateQueries({ queryKey: [key] })
      )
    )
  const save = useMutation({
    mutationFn: () =>
      write(
        `/api/oauth-apps/${entry.slug}`,
        "PUT",
        { clientId, clientSecret },
        nothing
      ),
    onSuccess: async () => {
      setClientId("")
      setClientSecret("")
      await refresh()
      setEditing(false)
      report("OAuth app saved. You can now connect at the provider.")
    },
    onError: (error) => report(message(error), "error"),
  })
  if (configured && !editing)
    return (
      <section className="oauth-app-config">
        <p>
          OAuth app configured. Its client ID and secret are sealed in your
          tenant.
        </p>
        <div className="detail-actions">
          <Button
            variant="outline"
            disabled={save.isPending}
            onClick={() => setEditing(true)}
          >
            Replace OAuth app
          </Button>
          <ConfirmAction
            label="Remove OAuth app"
            description="Future links will need app configuration. Existing linked accounts remain available."
            onConfirm={async () => {
              await write(
                `/api/oauth-apps/${entry.slug}`,
                "DELETE",
                undefined,
                nothing
              )
              await refresh()
            }}
          />
        </div>
      </section>
    )
  return (
    <form
      className="connection-form oauth-app-config"
      onSubmit={(event) => {
        event.preventDefault()
        save.mutate()
      }}
    >
      <p>
        Register your own OAuth app at {entry.name}, using this callback URL.
        Then paste its client ID and secret below.
      </p>
      <Facts
        items={[
          {
            label: "Callback URL",
            value: (
              <code>https://{config.hostname}/api/accounts/oauth/callback</code>
            ),
          },
          {
            label: "Requested scopes",
            value: <code>{entry.oauth?.scopes.join(" ")}</code>,
          },
        ]}
      />
      <div className="credential-field">
        <label htmlFor="app-client-id">Client ID</label>
        <Input
          id="app-client-id"
          autoComplete="off"
          required
          maxLength={512}
          value={clientId}
          onChange={(e) => setClientId(e.target.value)}
          disabled={save.isPending}
        />
      </div>
      <div className="credential-field">
        <label htmlFor="app-client-secret">Client secret</label>
        <Input
          id="app-client-secret"
          type="password"
          autoComplete="new-password"
          required
          maxLength={4096}
          value={clientSecret}
          onChange={(e) => setClientSecret(e.target.value)}
          disabled={save.isPending}
          aria-describedby="app-secret-hint"
        />
        <span id="app-secret-hint" className="form-hint">
          Stored values are never shown in this form again.
        </span>
      </div>
      {save.error && (
        <p className="connection-error" role="alert">
          {message(save.error)}
        </p>
      )}
      <div className="detail-actions">
        <Button type="submit" disabled={save.isPending}>
          {save.isPending ? "Saving…" : "Save OAuth app"}
        </Button>
        {configured && (
          <Button
            variant="ghost"
            type="button"
            disabled={save.isPending}
            onClick={() => {
              setClientId("")
              setClientSecret("")
              setEditing(false)
            }}
          >
            Cancel
          </Button>
        )}
      </div>
    </form>
  )
}
