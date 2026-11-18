import { z } from "zod"
export class ApiError extends Error {
  status: number
  constructor(status: number, message: string) {
    super(message)
    this.status = status
  }
}
export async function api<T>(
  path: `/api/${string}`,
  schema: z.ZodType<T>,
  init?: RequestInit
): Promise<T> {
  const response = await fetch(path, { credentials: "same-origin", ...init })
  if (!response.ok) {
    const error = z
      .object({ error: z.string() })
      .safeParse(await response.json().catch(() => undefined))
    throw new ApiError(
      response.status,
      error.success
        ? error.data.error
        : `Request failed (${response.status}). Try again.`
    )
  }
  const text = await response.text()
  const body: unknown = text ? JSON.parse(text) : undefined
  return schema.parse(body)
}
export function write<T>(
  path: `/api/${string}`,
  method: "POST" | "PUT" | "PATCH" | "DELETE",
  body: unknown,
  schema: z.ZodType<T>
) {
  return api(path, schema, {
    method,
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  })
}
export const nothing = z.void()
export const created = z.object({ id: z.uuid() })
export const bulkResult = z.object({ changed: z.number() })
export const httpUrl = z
  .url()
  .refine(
    (value) => ["http:", "https:"].includes(new URL(value).protocol),
    "Expected an HTTP(S) address"
  )
export const redirectResponse = z.object({ redirectUri: httpUrl })
export function message(error: unknown) {
  return error instanceof Error
    ? error.message
    : "Something went wrong. Try again."
}
export function safeReturnPath(value: unknown) {
  if (
    typeof value !== "string" ||
    !value.startsWith("/") ||
    value.startsWith("//") ||
    value.includes("\\") ||
    Array.from(value).some((character) => character.charCodeAt(0) <= 32)
  )
    return "/sources"
  const url = new URL(value, "https://return.invalid")
  if (url.pathname === "/executions" || url.pathname === "/executions/")
    return "/sessions"
  if (
    url.origin !== "https://return.invalid" ||
    ![
      "/",
      "/integrations",
      "/sources",
      "/accounts",
      "/workspaces",
      "/endpoints",
      "/tokens",
      "/executions",
      "/sessions",
      "/scripts",
      "/authorize",
    ].some((path) =>
      path === "/"
        ? url.pathname === "/"
        : url.pathname === path || url.pathname.startsWith(`${path}/`)
    )
  )
    return "/sources"
  return value
}
