import { useState } from "react"
import { createFileRoute, useRouter } from "@tanstack/react-router"
import {
  useMutation,
  useQueryClient,
  useSuspenseQuery,
} from "@tanstack/react-query"
import { z } from "zod"
import { Button, buttonVariants } from "@/components/ui/button"
import {
  Field,
  FieldGroup,
  FieldLabel,
  FieldDescription,
} from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import { ErrorNotice } from "@/components/common"
import { safeReturnPath, write } from "@/lib/api"
import { configQuery, sessionQuery, sessionSchema } from "@/lib/runtime"
export const Route = createFileRoute("/login")({
  validateSearch: z.object({
    redirect: z.unknown().optional().transform(safeReturnPath),
  }),
  head: () => ({ meta: [{ title: "Sign in · Codemode" }] }),
  component: Login,
})
function Login() {
  const config = useSuspenseQuery(configQuery).data
  const session = useSuspenseQuery(sessionQuery).data
  const [creating, setCreating] = useState(session.firstUser)
  const [email, setEmail] = useState("")
  const { redirect } = Route.useSearch()
  const router = useRouter()
  const cache = useQueryClient()
  const [password, setPassword] = useState("")
  const login = useMutation({
    mutationFn: () =>
      write(
        creating ? "/api/register" : "/api/session",
        "POST",
        { email, password },
        sessionSchema
      ),
    onSuccess: async (session) => {
      setPassword("")
      cache.clear()
      cache.setQueryData(sessionQuery.queryKey, session)
      await router.navigate({ href: redirect })
    },
  })
  if (config.idEnabled) {
    return (
      <main className="auth-page">
        <section className="auth-panel">
          <div className="flex flex-col gap-2">
            <p className="font-semibold">Codemode</p>
            <h1>Sign in to your control plane</h1>
            <p className="text-muted-foreground">
              Use your Microapps ID. Your MCPs, connected accounts and agent
              access stay in one private workspace.
            </p>
          </div>
          <a
            className={buttonVariants()}
            href={`/auth/login?return_to=${encodeURIComponent(redirect)}`}
          >
            Sign in with Microapps ID
          </a>
          <p className="text-sm break-all text-muted-foreground">
            {config.hostname}
          </p>
        </section>
      </main>
    )
  }
  return (
    <main className="auth-page">
      <section className="auth-panel">
        <div className="flex flex-col gap-2">
          <p className="font-semibold">Codemode</p>
          <h1>
            {creating ? "Create your account" : "Sign in to your control plane"}
          </h1>
          <p className="text-muted-foreground">
            Your MCPs, connected accounts and agent access in one private
            workspace.
          </p>
        </div>
        <form
          onSubmit={(event) => {
            event.preventDefault()
            login.mutate()
          }}
        >
          <FieldGroup>
            <Field>
              <FieldLabel htmlFor="email">Email</FieldLabel>
              <Input
                id="email"
                name="email"
                type="email"
                autoComplete="username"
                value={email}
                onChange={(e) => setEmail(e.target.value)}
                required
                maxLength={254}
              />
            </Field>
            <Field data-invalid={!!login.error}>
              <FieldLabel htmlFor="password">Password</FieldLabel>
              <Input
                id="password"
                name="password"
                type="password"
                autoComplete={creating ? "new-password" : "current-password"}
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                required
                maxLength={1024}
                minLength={creating ? 12 : undefined}
                aria-invalid={!!login.error}
              />
              <FieldDescription>
                {creating
                  ? "Use at least 12 characters."
                  : "Use the password for your email address."}
              </FieldDescription>
            </Field>
            <ErrorNotice error={login.error} />
            <Button type="submit" disabled={login.isPending}>
              {login.isPending
                ? creating
                  ? "Creating account…"
                  : "Signing in…"
                : creating
                  ? "Create account"
                  : "Sign in"}
            </Button>
          </FieldGroup>
        </form>
        {creating && (
          <p className="text-sm text-muted-foreground">
            Registration is open to ursus95@gmail.com and @olegpustovit.com
            addresses.
          </p>
        )}
        {creating && session.firstUser && (
          <p className="text-sm text-muted-foreground">
            The first account receives the admin attribute. Every account has
            its own workspace.
          </p>
        )}
        <Button
          variant="ghost"
          onClick={() => {
            setCreating(!creating)
            login.reset()
            setPassword("")
          }}
        >
          {creating ? "Already registered? Sign in" : "Create an account"}
        </Button>
        <p className="text-sm break-all text-muted-foreground">
          {config.hostname}
        </p>
      </section>
    </main>
  )
}
