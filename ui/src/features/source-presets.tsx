import {
  Card,
  CardHeader,
  CardTitle,
  CardDescription,
  CardContent,
  CardFooter,
} from "@/components/ui/card"
import {
  useMutation,
  useQueryClient,
  useSuspenseQuery,
} from "@tanstack/react-query"
import { Link, useRouter } from "@tanstack/react-router"
import { Plus } from "lucide-react"
import { sourcesQuery } from "./queries"
import { created, write } from "@/lib/api"
import { ErrorNotice } from "@/components/common"
import { Button } from "@/components/ui/button"

const presets = [
  {
    name: "GitHub",
    slug: "github",
    url: "https://api.githubcopilot.com/mcp/",
    description: "Repositories, issues and pull requests.",
    docs: "https://github.com/github/github-mcp-server/blob/main/docs/remote-server.md",
  },
  {
    name: "Notion",
    slug: "notion",
    url: "https://mcp.notion.com/mcp",
    description: "Pages, documents and databases.",
    docs: "https://developers.notion.com/guides/mcp/get-started-with-mcp",
  },
  {
    name: "Linear",
    slug: "linear",
    url: "https://mcp.linear.app/mcp",
    description: "Issues, projects and team workflows.",
    docs: "https://linear.app/docs/mcp",
  },
  {
    name: "Sentry",
    slug: "sentry",
    url: "https://mcp.sentry.dev/mcp",
    description: "Errors, performance and issue triage.",
    docs: "https://mcp.sentry.dev/",
  },
]
export function SourcePresets() {
  const sources = useSuspenseQuery(sourcesQuery).data
  const cache = useQueryClient()
  const router = useRouter()
  const add = useMutation({
    mutationFn: (preset: (typeof presets)[number]) =>
      write(
        "/api/sources",
        "POST",
        {
          slug: preset.slug,
          kind: "mcp_http",
          url: preset.url,
          requiresAccount: true,
        },
        created
      ),
    onSuccess: async ({ id }) => {
      await cache.invalidateQueries({ queryKey: ["sources"] })
      await router.navigate({
        to: "/sources/$sourceId/accounts",
        params: { sourceId: id },
      })
    },
  })
  return (
    <section className="section-rule flex flex-col gap-4">
      <div>
        <h2>Popular MCP servers</h2>
        <p className="mt-2 text-sm text-muted-foreground">
          Start with a preset, then link your account. You choose what to
          authorize with the provider.
        </p>
      </div>
      <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
        {presets.map((preset) => {
          const existing = sources.find(
            (s) => s.kind === "mcp_http" && s.url === preset.url
          )
          const nameTaken = sources.some((s) => s.slug === preset.slug)
          return (
            <Card key={preset.slug}>
              <CardHeader>
                <CardTitle>{preset.name}</CardTitle>
                <CardDescription>{preset.description}</CardDescription>
              </CardHeader>
              <CardContent className="mt-auto">
                <a
                  className="mt-auto text-sm underline underline-offset-4"
                  href={preset.docs}
                  target="_blank"
                  rel="noreferrer"
                >
                  Provider documentation
                </a>
              </CardContent>
              <CardFooter>
                {existing ? (
                  <Button
                    variant="outline"
                    render={
                      <Link
                        to="/sources/$sourceId/accounts"
                        params={{ sourceId: existing.id }}
                      />
                    }
                  >
                    Manage connection
                  </Button>
                ) : (
                  <Button
                    variant="outline"
                    disabled={add.isPending || nameTaken}
                    onClick={() => add.mutate(preset)}
                  >
                    <Plus data-icon="inline-start" />
                    {add.isPending && add.variables.slug === preset.slug
                      ? "Adding…"
                      : nameTaken
                        ? "Name already used"
                        : `Add ${preset.name}`}
                  </Button>
                )}
              </CardFooter>
            </Card>
          )
        })}
      </div>
      <ErrorNotice error={add.error} />
    </section>
  )
}
