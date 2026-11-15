import { createFileRoute, Link } from "@tanstack/react-router"
import { useSuspenseQuery } from "@tanstack/react-query"
import { sessionQuery } from "@/lib/runtime"
import { PageHeader } from "@/components/common"
import { Facts } from "@/components/facts"
import { Button } from "@/components/ui/button"

export const Route = createFileRoute("/_authenticated/account")({
  head: () => ({ meta: [{ title: "Account settings · Codemode" }] }),
  component: AccountSettings,
})
function AccountSettings() {
  const { user } = useSuspenseQuery(sessionQuery).data
  return (
    <>
      <PageHeader
        title="Account settings"
        description="Your private Codemode account."
      />
      <Facts
        items={[
          { label: "Email", value: user?.email },
          { label: "Account", value: user?.admin ? "Administrator" : "Member" },
        ]}
      />
      <Button render={<Link to="/accounts" />}>Linked accounts</Button>
    </>
  )
}
