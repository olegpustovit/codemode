import { queryOptions } from "@tanstack/react-query"
import { z } from "zod"
import { api } from "./api"
export const sessionSchema = z.object({
  authenticated: z.boolean(),
  firstUser: z.boolean(),
  user: z
    .object({
      id: z.string(),
      email: z.string(),
      tenantId: z.string(),
      admin: z.boolean(),
    })
    .nullable(),
})
export const sessionQuery = queryOptions({
  queryKey: ["session"],
  queryFn: () => api("/api/session", sessionSchema),
  staleTime: 0,
})
export const configQuery = queryOptions({
  queryKey: ["config"],
  queryFn: () =>
    api(
      "/api/config",
      z.object({
        name: z.string(),
        hostname: z.string(),
        loginEnabled: z.boolean(),
      })
    ),
  staleTime: Infinity,
})
