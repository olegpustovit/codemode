import { queryOptions, infiniteQueryOptions } from "@tanstack/react-query"
import { z } from "zod"
import { api } from "@/lib/api"
const common = {
  id: z.uuid(),
  slug: z.string(),
  name: z.string(),
  icon: z.string(),
  requiresAccount: z.boolean(),
  status: z.string(),
  lastError: z.string().nullable(),
  syncedAt: z.string().nullable(),
  toolCount: z.number(),
  revision: z.number(),
  pythonName: z.string(),
}
export const sourceSchema = z.discriminatedUnion("kind", [
  z.object({ ...common, kind: z.literal("mcp_http"), url: z.string() }),
  z.object({ ...common, kind: z.literal("graphql"), url: z.string() }),
  z.object({
    ...common,
    kind: z.literal("openapi"),
    url: z.string(),
    serverUrl: z.string(),
  }),
  z.object({
    ...common,
    kind: z.literal("mcp_stdio"),
    command: z.string(),
    args: z.array(z.string()),
  }),
])
export type Source = z.infer<typeof sourceSchema>
export const workspaceSchema = z.object({
  id: z.uuid(),
  name: z.string(),
  allSources: z.boolean(),
  allTools: z.boolean(),
  sourceIds: z.array(z.uuid()),
})
export type Workspace = z.infer<typeof workspaceSchema>
export const accountSchema = z.object({
  sourceId: z.uuid(),
  kind: z.string(),
  keyId: z.string(),
  expiresAt: z.string().nullable(),
})
export const tokenSchema = z.object({
  oauth: z.boolean(),
  lastUsedAt: z.string().nullable(),
  id: z.uuid(),
  name: z.string(),
  audience: z.string(),
  workspaceId: z.uuid().nullable(),
  resource: z.enum(["/mcp", "/mcp-admin"]),
  revoked: z.boolean(),
})
const summary = z.object({
  id: z.uuid(),
  tokenId: z.uuid(),
  sessionId: z.uuid().nullable(),
  title: z.string().nullable(),
  status: z.string(),
  error: z.string().nullable(),
  toolCalls: z.number(),
  startedAt: z.string(),
  finishedAt: z.string().nullable(),
})
const detail = summary.extend({
  code: z.string(),
  result: z.string().nullable(),
  output: z.string().nullable(),
  calls: z.array(
    z.object({
      seq: z.number(),
      path: z.string(),
      argsRedacted: z.unknown(),
      outcome: z.string(),
      error: z.string().nullable(),
      startedAt: z.string(),
      durationMs: z.number(),
      preview: z.string().nullable(),
    })
  ),
})
export type ExecutionDetail = z.infer<typeof detail>
export const sourcesQuery = queryOptions({
  queryKey: ["sources"],
  queryFn: () => api("/api/sources", sourceSchema.array()),
  refetchInterval: 10_000,
})
export const sourceQuery = (id: string) =>
  queryOptions({
    queryKey: ["sources", id],
    queryFn: () => api(`/api/sources/${id}`, sourceSchema),
    refetchInterval: 10_000,
  })
export const workspacesQuery = queryOptions({
  queryKey: ["workspaces"],
  queryFn: () => api("/api/workspaces", workspaceSchema.array()),
})
export const workspaceQuery = (id: string) =>
  queryOptions({
    queryKey: ["workspaces", id],
    queryFn: () => api(`/api/workspaces/${id}`, workspaceSchema),
  })
export const accountsQuery = queryOptions({
  queryKey: ["accounts"],
  queryFn: () => api("/api/accounts", accountSchema.array()),
})
export const tokensQuery = queryOptions({
  queryKey: ["tokens"],
  queryFn: () => api("/api/tokens", tokenSchema.array()),
})
export const executionQuery = (id: string) =>
  queryOptions({
    queryKey: ["executions", id],
    queryFn: () => api(`/api/executions/${id}`, detail),
    refetchInterval: (query) =>
      query.state.data?.status === "running" ? 2_000 : false,
  })
const policyAction = z.enum(["allow", "block"])
export const toolPolicySchema = z.object({
  signature: z.string(),
  sourceId: z.uuid(),
  sourceRevision: z.number(),
  upstreamName: z.string(),
  path: z.string(),
  description: z.string(),
  readOnly: z.boolean(),
  destructive: z.boolean(),
  inScope: z.boolean(),
  overrideAction: policyAction.nullable(),
  effectiveAction: policyAction,
})
export type ToolPolicy = z.infer<typeof toolPolicySchema>
const policyPageSchema = z.object({
  tools: toolPolicySchema.array(),
  totalMatches: z.number(),
  nextCursor: z.string().nullable(),
})
const loadPolicies = (id: string, q: string, cursor?: string) => {
  const search = new URLSearchParams({ q })
  if (cursor) search.set("cursor", cursor)
  return api(`/api/workspaces/${id}/policies?${search}`, policyPageSchema)
}
export const policyQuery = (id: string, q = "", cursor?: string) =>
  queryOptions({
    queryKey: ["policies", id, { q, cursor }],
    queryFn: () => loadPolicies(id, q, cursor),
  })
export const policyInfiniteQuery = (id: string, q = "") =>
  infiniteQueryOptions({
    queryKey: ["policies", id, "infinite", { q }],
    initialPageParam: "",
    queryFn: ({ pageParam }) => loadPolicies(id, q, pageParam),
    getNextPageParam: (page) => page.nextCursor ?? undefined,
  })
export const scriptSummarySchema = z.object({
  id: z.uuid(),
  name: z.string(),
  description: z.string(),
  revision: z.number(),
  updatedAt: z.string(),
})
export const scriptSchema = scriptSummarySchema.extend({ code: z.string() })
export type Script = z.infer<typeof scriptSchema>
export const scriptsQuery = (workspace: string) =>
  queryOptions({
    queryKey: ["scripts", workspace],
    queryFn: () =>
      api(`/api/workspaces/${workspace}/scripts`, scriptSummarySchema.array()),
  })
export const scriptQuery = (workspace: string, id: string) =>
  queryOptions({
    queryKey: ["scripts", workspace, id],
    queryFn: () =>
      api(`/api/workspaces/${workspace}/scripts/${id}`, scriptSchema),
  })

export const integrationSchema = z.object({
  slug: z.string(),
  name: z.string(),
  category: z.string(),
  description: z.string(),
  url: z.string(),
  docs: z.string(),
  icon: z.string(),
  methods: z.array(
    z.enum(["one_click", "oauth_app", "api_key", "open", "local"])
  ),
  oauth: z
    .object({
      authorizationEndpoint: z.string(),
      tokenEndpoint: z.string(),
      scopes: z.array(z.string()),
    })
    .optional(),
  connections: z
    .array(
      z.object({
        sourceId: z.uuid(),
        slug: z.string(),
        name: z.string(),
        state: z.string(),
        toolCount: z.number(),
        error: z.string().nullable(),
      })
    )
    .default([]),
  sourceId: z.uuid().nullable(),
  state: z.string(),
  toolCount: z.number(),
  toolNames: z.array(z.string()).default([]),
  error: z.string().nullable(),
})
export type Integration = z.infer<typeof integrationSchema>
export const integrationsQuery = queryOptions({
  queryKey: ["integrations"],
  queryFn: () => api("/api/integrations", integrationSchema.array()),
  refetchInterval: 10_000,
})

export const oauthAppsQuery = queryOptions({
  queryKey: ["oauth-apps"],
  queryFn: () => api("/api/oauth-apps", z.array(z.string())),
})

export const agentSessionSchema = z.object({
  id: z.uuid(),
  tokenId: z.uuid(),
  workspaceId: z.uuid(),
  workspaceName: z.string(),
  title: z.string(),
  client: z.string(),
  startedAt: z.string(),
  lastSeenAt: z.string(),
  expiresAt: z.string(),
  executions: z.number(),
  toolCalls: z.number(),
  failures: z.number(),
  status: z.string(),
  sources: z.string().array(),
})
export type AgentSession = z.infer<typeof agentSessionSchema>
const agentSessionPageSchema = z.object({
  sessions: agentSessionSchema.array(),
  totalMatches: z.number(),
  allMatches: z.number(),
  errorMatches: z.number(),
  nextCursor: z.uuid().nullable(),
})
const loadAgentSessions = (q: string, before?: string, errorsOnly = false) => {
  const search = new URLSearchParams({ q, errors_only: String(errorsOnly) })
  if (before) search.set("before", before)
  return api(`/api/sessions?${search}`, agentSessionPageSchema)
}
export const agentSessionsQuery = (q = "", before?: string) =>
  queryOptions({
    queryKey: ["agent-sessions", { q, before }],
    queryFn: () => loadAgentSessions(q, before),
    refetchInterval: 5_000,
  })
export const agentSessionsInfiniteQuery = (q = "", errorsOnly = false) =>
  infiniteQueryOptions({
    queryKey: ["agent-sessions", "infinite", { q, errorsOnly }],
    initialPageParam: "",
    queryFn: ({ pageParam }) => loadAgentSessions(q, pageParam, errorsOnly),
    getNextPageParam: (page) => page.nextCursor ?? undefined,
    refetchInterval: 5_000,
  })
const agentSessionDetailSchema = z.object({
  session: agentSessionSchema,
  executions: detail.array(),
  nextCursor: z.uuid().nullable(),
})
const loadAgentSession = (id: string, before?: string) =>
  api(
    `/api/sessions/${id}${before ? `?before=${before}` : ""}`,
    agentSessionDetailSchema
  )
export const agentSessionQuery = (id: string, before?: string) =>
  queryOptions({
    queryKey: ["agent-sessions", id, { before }],
    queryFn: () => loadAgentSession(id, before),
    refetchInterval: 5_000,
  })
export const agentSessionInfiniteQuery = (id: string) =>
  infiniteQueryOptions({
    queryKey: ["agent-sessions", id, "infinite"],
    initialPageParam: "",
    queryFn: ({ pageParam }) => loadAgentSession(id, pageParam),
    getNextPageParam: (page) => page.nextCursor ?? undefined,
    refetchInterval: 5_000,
  })
