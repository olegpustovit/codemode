import { queryOptions } from "@tanstack/react-query"
import { z } from "zod"
import { api } from "@/lib/api"
export const authorizationSearch = z.object({
  response_type: z.string().catch(""),
  client_id: z.string().catch(""),
  redirect_uri: z.string().catch(""),
  resource: z.string().catch(""),
  code_challenge: z.string().catch(""),
  code_challenge_method: z.string().catch(""),
  state: z.string().max(2048).optional().catch(undefined),
  scope: z.string().optional().catch(undefined),
})
export const consentQuery = (request: z.infer<typeof authorizationSearch>) =>
  queryOptions({
    queryKey: ["oauth-consent", request],
    queryFn: () => {
      const query = new URLSearchParams()
      for (const [key, value] of Object.entries(request))
        if (typeof value === "string") query.set(key, value)
      return api(
        `/api/oauth/authorize?${query}`,
        z.object({
          clientId: z.string(),
          clientName: z.string(),
          redirectUri: z.string(),
          resource: z.string(),
          scope: z.string(),
          workspaceId: z.uuid().nullable(),
        })
      )
    },
    staleTime: 0,
    retry: false,
  })
