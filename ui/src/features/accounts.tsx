import { useState } from "react"
import { Link, getRouteApi } from "@tanstack/react-router"
import {
  useMutation,
  useQueryClient,
  useSuspenseQuery,
} from "@tanstack/react-query"
import { z } from "zod"
import { ArrowLeft, ExternalLink, Plus, X } from "lucide-react"
import { sourcesQuery, sourceQuery, accountsQuery } from "./queries"
import { write, nothing, httpUrl } from "@/lib/api"
import { PageHeader, Blank, ErrorNotice, date } from "@/components/common"
import { ConfirmAction } from "@/components/confirm-action"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import {
  Field,
  FieldGroup,
  FieldLabel,
  FieldDescription,
  FieldSet,
  FieldLegend,
} from "@/components/ui/field"
import {
  Table,
  TableHeader,
  TableHead,
  TableBody,
  TableRow,
  TableCell,
} from "@/components/ui/table"
import { Alert, AlertTitle, AlertDescription } from "@/components/ui/alert"
import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select"
import { toast } from "@/components/ui/toast"
const accountRoute = getRouteApi("/_authenticated/sources/$sourceId_/accounts")
export function AccountsPage() {
  const sources = useSuspenseQuery(sourcesQuery).data
  const accounts = useSuspenseQuery(accountsQuery).data
  return (
    <>
      <PageHeader
        title="Linked accounts"
        description="Credentials stay in the gateway. Agents receive tool results."
      />
      {!sources.length ? (
        <Blank
          title="Add a source first"
          description="Each source has its own linked credential."
        >
          <Button render={<Link to="/sources/new" />}>Add source</Button>
        </Blank>
      ) : (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Source</TableHead>
              <TableHead>Account</TableHead>
              <TableHead>Access token expiry</TableHead>
              <TableHead>
                <span className="sr-only">Manage</span>
              </TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {sources.map((source) => {
              const account = accounts.find((a) => a.sourceId === source.id)
              return (
                <TableRow key={source.id}>
                  <TableCell>{source.slug}</TableCell>
                  <TableCell>
                    {account
                      ? account.kind === "environment"
                        ? "Environment variables"
                        : account.kind === "oauth"
                          ? "OAuth"
                          : account.kind === "rest"
                            ? "HTTP credential"
                            : "Bearer token"
                      : source.requiresAccount
                        ? "Not linked"
                        : "Optional"}
                  </TableCell>
                  <TableCell>{date(account?.expiresAt ?? null)}</TableCell>
                  <TableCell>
                    <Button
                      variant="outline"
                      size="sm"
                      render={
                        <Link
                          to="/sources/$sourceId/accounts"
                          params={{ sourceId: source.id }}
                        />
                      }
                    >
                      {account ? "Manage" : "Link account"}
                    </Button>
                  </TableCell>
                </TableRow>
              )
            })}
          </TableBody>
        </Table>
      )}
    </>
  )
}
export function AccountPage() {
  const { sourceId } = accountRoute.useParams()
  return <AccountEditor key={sourceId} sourceId={sourceId} />
}
function AccountEditor({ sourceId }: { sourceId: string }) {
  const { oauth } = accountRoute.useSearch()
  const source = useSuspenseQuery(sourceQuery(sourceId)).data
  const httpApi = source.kind === "openapi" || source.kind === "graphql"
  const accounts = useSuspenseQuery(accountsQuery).data
  const linked = accounts.find((account) => account.sourceId === sourceId)
  const cache = useQueryClient()
  const [credential, setCredential] = useState("")
  const [httpKind, setHttpKind] = useState<
    "bearer" | "api_key_header" | "api_key_query" | "basic"
  >("bearer")
  const [credentialName, setCredentialName] = useState("")
  const [username, setUsername] = useState("")
  const [variables, setVariables] = useState([
    { id: "initial", name: "", value: "" },
  ])
  const refresh = () =>
    Promise.all([
      cache.invalidateQueries({ queryKey: ["accounts"] }),
      cache.invalidateQueries({ queryKey: ["sources"] }),
      cache.invalidateQueries({ queryKey: ["policies"] }),
    ])
  const save = useMutation({
    mutationFn: async () => {
      if (httpApi)
        return write(
          `/api/accounts/${sourceId}/http`,
          "PUT",
          {
            kind: httpKind,
            ...(httpKind === "basic"
              ? { username, password: z.string().max(65536).parse(credential) }
              : {
                  credential: z.string().min(1).max(65536).parse(credential),
                  ...(httpKind === "bearer"
                    ? {}
                    : {
                        name: z.string().min(1).max(256).parse(credentialName),
                      }),
                }),
          },
          nothing
        )
      if (source.kind === "mcp_http")
        return write(
          `/api/accounts/${sourceId}`,
          "PUT",
          { credential: z.string().min(1).max(65536).parse(credential) },
          nothing
        )
      const names = variables.map((v) =>
        z
          .string()
          .regex(/^[_A-Za-z][_A-Za-z0-9]*$/)
          .parse(v.name)
      )
      if (new Set(names).size !== names.length)
        throw new Error("Environment variable names must be unique.")
      return write(
        `/api/accounts/${sourceId}/environment`,
        "PUT",
        {
          environment: Object.fromEntries(
            variables.map((v) => [v.name, v.value])
          ),
        },
        nothing
      )
    },
    onSuccess: async () => {
      setCredential("")
      setUsername("")
      setCredentialName("")
      setVariables([{ id: "initial", name: "", value: "" }])
      await refresh()
      toast.add({ title: "Account linked", type: "success" })
    },
  })
  const connect = useMutation({
    mutationFn: () =>
      write(
        `/api/accounts/${sourceId}/oauth`,
        "POST",
        undefined,
        z.object({ authorizationUrl: httpUrl })
      ),
    onSuccess: (data) => window.location.assign(data.authorizationUrl),
  })
  return (
    <>
      <Link
        to="/sources/$sourceId"
        params={{ sourceId }}
        className="flex items-center gap-2 text-sm text-muted-foreground"
      >
        <ArrowLeft className="size-4" />
        {source.slug}
      </Link>
      <PageHeader
        title={`Account for ${source.slug}`}
        description={
          source.kind === "mcp_stdio"
            ? source.command
            : source.kind === "openapi"
              ? source.serverUrl
              : source.url
        }
      />
      {oauth === "denied" && (
        <Alert>
          <AlertTitle>Connection cancelled</AlertTitle>
          <AlertDescription>
            You can try again or enter a bearer token.
          </AlertDescription>
        </Alert>
      )}
      <div className="record-grid">
        <section className="flex flex-col gap-7">
          {source.kind === "mcp_http" && (
            <section className="section-rule flex flex-col gap-3">
              <h2>Connect with OAuth</h2>
              <p className="text-sm text-muted-foreground">
                Authorize this gateway at the provider. Access tokens refresh
                automatically when the provider supports it.
              </p>
              <Button
                className="self-start"
                onClick={() => connect.mutate()}
                disabled={connect.isPending}
              >
                <ExternalLink data-icon="inline-start" />
                {connect.isPending
                  ? "Preparing connection…"
                  : linked
                    ? "Reconnect with OAuth"
                    : "Connect with OAuth"}
              </Button>
              <ErrorNotice error={connect.error} />
            </section>
          )}
          <form
            onSubmit={(event) => {
              event.preventDefault()
              save.mutate()
            }}
          >
            <FieldGroup>
              {source.kind !== "mcp_stdio" ? (
                <>
                  {httpApi && (
                    <>
                      <Field>
                        <FieldLabel htmlFor="http-kind">
                          Credential type
                        </FieldLabel>
                        <NativeSelect
                          id="http-kind"
                          value={httpKind}
                          onChange={(event) =>
                            setHttpKind(
                              z
                                .enum([
                                  "bearer",
                                  "api_key_header",
                                  "api_key_query",
                                  "basic",
                                ])
                                .parse(event.target.value)
                            )
                          }
                        >
                          <NativeSelectOption value="bearer">
                            Bearer token
                          </NativeSelectOption>
                          <NativeSelectOption value="api_key_header">
                            API key in a header
                          </NativeSelectOption>
                          <NativeSelectOption value="api_key_query">
                            API key in the query
                          </NativeSelectOption>
                          <NativeSelectOption value="basic">
                            Basic authentication
                          </NativeSelectOption>
                        </NativeSelect>
                      </Field>
                      {(httpKind === "api_key_header" ||
                        httpKind === "api_key_query") && (
                        <Field>
                          <FieldLabel htmlFor="credential-name">
                            {httpKind === "api_key_header"
                              ? "Header name"
                              : "Query parameter name"}
                          </FieldLabel>
                          <Input
                            id="credential-name"
                            value={credentialName}
                            onChange={(event) =>
                              setCredentialName(event.target.value)
                            }
                            required
                            maxLength={256}
                          />
                        </Field>
                      )}
                      {httpKind === "basic" && (
                        <Field>
                          <FieldLabel htmlFor="username">Username</FieldLabel>
                          <Input
                            id="username"
                            autoComplete="off"
                            value={username}
                            onChange={(event) =>
                              setUsername(event.target.value)
                            }
                            maxLength={1024}
                          />
                        </Field>
                      )}
                    </>
                  )}
                  <Field>
                    <FieldLabel htmlFor="credential">
                      {httpApi
                        ? httpKind === "basic"
                          ? "Password"
                          : httpKind === "bearer"
                            ? "Bearer token"
                            : "API key"
                        : linked
                          ? "Replace bearer token"
                          : "Bearer token"}
                    </FieldLabel>
                    <Input
                      id="credential"
                      type="password"
                      autoComplete="new-password"
                      value={credential}
                      onChange={(e) => setCredential(e.target.value)}
                      required={!httpApi || httpKind !== "basic"}
                      maxLength={65536}
                    />
                    <FieldDescription>
                      {httpApi
                        ? "The gateway adds this credential when it calls the saved API server. The stored value is never displayed."
                        : "Use this when the provider supplies a token instead of OAuth. The stored value is never displayed."}
                    </FieldDescription>
                  </Field>
                </>
              ) : (
                <FieldSet>
                  <FieldLegend>Environment variables</FieldLegend>
                  <FieldDescription>
                    Only these variables are passed to the stdio server.
                  </FieldDescription>
                  {variables.map((variable, index) => (
                    <div key={variable.id} className="flex items-end gap-2">
                      <Field>
                        <FieldLabel htmlFor={`name-${variable.id}`}>
                          Name {index + 1}
                        </FieldLabel>
                        <Input
                          id={`name-${variable.id}`}
                          value={variable.name}
                          onChange={(e) =>
                            setVariables(
                              variables.map((v) =>
                                v.id === variable.id
                                  ? { ...v, name: e.target.value }
                                  : v
                              )
                            )
                          }
                          required
                          pattern="[_A-Za-z][_A-Za-z0-9]*"
                        />
                      </Field>
                      <Field>
                        <FieldLabel htmlFor={`value-${variable.id}`}>
                          Value {index + 1}
                        </FieldLabel>
                        <Input
                          id={`value-${variable.id}`}
                          type="password"
                          autoComplete="new-password"
                          value={variable.value}
                          onChange={(e) =>
                            setVariables(
                              variables.map((v) =>
                                v.id === variable.id
                                  ? { ...v, value: e.target.value }
                                  : v
                              )
                            )
                          }
                        />
                      </Field>
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon"
                        disabled={variables.length === 1}
                        aria-label={`Remove variable ${index + 1}`}
                        onClick={() =>
                          setVariables(
                            variables.filter((v) => v.id !== variable.id)
                          )
                        }
                      >
                        <X />
                      </Button>
                    </div>
                  ))}
                  <Button
                    type="button"
                    variant="outline"
                    className="self-start"
                    disabled={variables.length >= 64}
                    onClick={() =>
                      setVariables([
                        ...variables,
                        { id: crypto.randomUUID(), name: "", value: "" },
                      ])
                    }
                  >
                    <Plus data-icon="inline-start" />
                    Add variable
                  </Button>
                </FieldSet>
              )}
              <ErrorNotice error={save.error} />
              <Button type="submit" disabled={save.isPending}>
                {save.isPending ? "Saving…" : "Save credential"}
              </Button>
            </FieldGroup>
          </form>
        </section>
        <aside className="flex flex-col gap-4">
          <h2>Current account</h2>
          {linked ? (
            <>
              <p>
                {linked.kind === "oauth"
                  ? "OAuth connected"
                  : linked.kind === "environment"
                    ? "Environment linked"
                    : linked.kind === "rest"
                      ? "HTTP credential linked"
                      : "Bearer token linked"}
              </p>
              <p className="text-sm text-muted-foreground">
                Access expiry: {date(linked.expiresAt)}
              </p>
              <ConfirmAction
                label="Unlink account"
                description="Calls requiring this credential will stop until you link an account again."
                onConfirm={async () => {
                  await write(
                    `/api/accounts/${sourceId}`,
                    "DELETE",
                    undefined,
                    nothing
                  )
                  await refresh()
                }}
              />
            </>
          ) : (
            <p className="text-sm text-muted-foreground">
              No credential is linked.{" "}
              {source.requiresAccount
                ? "This source requires one before it can connect."
                : "This source can connect without credentials."}
            </p>
          )}
        </aside>
      </div>
    </>
  )
}
