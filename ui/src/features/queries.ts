import { queryOptions } from "@tanstack/react-query"
import { z } from "zod"
import { api } from "@/lib/api"
const common = {
  id: z.uuid(),
  slug: z.string(),
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
  status: z.string(),
  error: z.string().nullable(),
  toolCalls: z.number(),
  startedAt: z.string(),
  finishedAt: z.string().nullable(),
})
const detail = summary.extend({
  code: z.string(),
  calls: z.array(
    z.object({
      seq: z.number(),
      path: z.string(),
      argsRedacted: z.unknown(),
      outcome: z.string(),
      error: z.string().nullable(),
      startedAt: z.string(),
      durationMs: z.number(),
    })
  ),
})
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
export const executionsQuery = (before?: string) =>
  queryOptions({
    queryKey: ["executions", { before }],
    queryFn: () =>
      api(
        `/api/executions${before ? `?before=${encodeURIComponent(before)}` : ""}`,
        summary.array()
      ),
    refetchInterval: 5_000,
  })
export const executionQuery = (id: string) =>
  queryOptions({
    queryKey: ["executions", id],
    queryFn: () => api(`/api/executions/${id}`, detail),
    refetchInterval: (query) =>
      query.state.data?.status === "running" ? 2_000 : false,
  })
const policyAction = z.enum(["allow", "approve", "block"])
export const toolPolicySchema = z.object({
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
export const policyQuery = (id: string, q = "", cursor?: string) =>
  queryOptions({
    queryKey: ["policies", id, { q, cursor }],
    queryFn: () => {
      const search = new URLSearchParams({ q })
      if (cursor) search.set("cursor", cursor)
      return api(
        `/api/workspaces/${id}/policies?${search}`,
        z.object({
          tools: toolPolicySchema.array(),
          nextCursor: z.string().nullable(),
        })
      )
    },
  })
const approvalSummary = z.object({
  executionId: z.uuid(),
  hostname: z.string(),
  expiresAt: z.string(),
  pending: z.number(),
})
export const approvalsQuery = queryOptions({
  queryKey: ["approvals"],
  queryFn: () => api("/api/approvals", approvalSummary.array()),
  refetchInterval: 5_000,
})
export const approvalQuery = (id: string) =>
  queryOptions({
    queryKey: ["approvals", id],
    queryFn: () =>
      api(
        `/api/approvals/${id}`,
        z.object({
          executionId: z.uuid(),
          pauseId: z.uuid(),
          hostname: z.string(),
          code: z.string(),
          expiresAt: z.string(),
          calls: z.array(
            z.object({
              callId: z.number(),
              path: z.string(),
              args: z.unknown(),
              error: z.string().nullable(),
              decision: z.boolean().nullable(),
            })
          ),
        })
      ),
    refetchInterval: 5_000,
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
