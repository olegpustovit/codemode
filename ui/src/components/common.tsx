import type { ReactNode } from "react"
import { Link, useRouter } from "@tanstack/react-router"
import { useQueryErrorResetBoundary } from "@tanstack/react-query"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Empty,
  EmptyHeader,
  EmptyTitle,
  EmptyDescription,
  EmptyContent,
} from "@/components/ui/empty"
import { Skeleton } from "@/components/ui/skeleton"
import { ApiError, message, safeReturnPath } from "@/lib/api"
export function PageHeader({
  title,
  description,
  action,
}: {
  title: string
  description?: string
  action?: ReactNode
}) {
  return (
    <header className="page-heading">
      <div>
        <h1>{title}</h1>
        {description && <p className="text-muted-foreground">{description}</p>}
      </div>
      {action}
    </header>
  )
}
export function ErrorNotice({ error }: { error: unknown }) {
  return error ? (
    <Alert variant="destructive" role="alert">
      <AlertTitle>Action could not be completed</AlertTitle>
      <AlertDescription>{message(error)}</AlertDescription>
    </Alert>
  ) : null
}
export function StateBadge({ state }: { state: string }) {
  return (
    <Badge
      variant={
        ["error", "failed", "timed_out", "blocked"].includes(state)
          ? "destructive"
          : "outline"
      }
    >
      {state.replaceAll("_", " ")}
    </Badge>
  )
}
export function Blank({
  title,
  description,
  children,
}: {
  title: string
  description: string
  children?: ReactNode
}) {
  return (
    <Empty>
      <EmptyHeader>
        <EmptyTitle>{title}</EmptyTitle>
        <EmptyDescription>{description}</EmptyDescription>
      </EmptyHeader>
      {children && <EmptyContent>{children}</EmptyContent>}
    </Empty>
  )
}
export function Pending() {
  return (
    <div className="flex flex-col gap-4 p-8" role="status" aria-label="Loading">
      <Skeleton className="h-8 w-48" />
      <Skeleton className="h-40 w-full" />
    </div>
  )
}
export function RouteError({ error }: { error: unknown }) {
  const router = useRouter()
  const boundary = useQueryErrorResetBoundary()
  return (
    <main className="mx-auto flex max-w-xl flex-col gap-5 p-8">
      <h1>
        {error instanceof ApiError && error.status === 401
          ? "Your session has ended"
          : "This page could not load"}
      </h1>
      <p>{message(error)}</p>
      {error instanceof ApiError && error.status === 401 ? (
        <Button
          render={
            <Link
              to="/login"
              search={{ redirect: safeReturnPath(router.state.location.href) }}
            />
          }
        >
          Sign in
        </Button>
      ) : (
        <Button
          onClick={() => {
            boundary.reset()
            void router.invalidate()
          }}
        >
          Try again
        </Button>
      )}
    </main>
  )
}
export function NotFound() {
  return (
    <Blank
      title="Page not found"
      description="This address does not match a control-plane page."
    >
      <Button render={<Link to="/sources" />}>Go to sources</Button>
    </Blank>
  )
}
export function date(value: string | null) {
  return value
    ? new Intl.DateTimeFormat(undefined, {
        dateStyle: "medium",
        timeStyle: "short",
      }).format(new Date(value))
    : "—"
}
