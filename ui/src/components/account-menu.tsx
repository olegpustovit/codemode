import { Menu } from "@base-ui/react/menu"
import { Link } from "@tanstack/react-router"
import { ChevronDown } from "lucide-react"

export function AccountMenu({
  email,
  onLogout,
  busy = false,
}: {
  email: string
  onLogout: () => void
  busy?: boolean
}) {
  return (
    <Menu.Root>
      <Menu.Trigger
        className="account-menu-trigger"
        aria-label={`Account menu for ${email}`}
      >
        <span className="account-avatar">{email[0]?.toUpperCase()}</span>
        <span className="account-email">{email}</span>
        <ChevronDown size={12} aria-hidden="true" />
      </Menu.Trigger>
      <Menu.Portal>
        <Menu.Positioner
          align="end"
          sideOffset={6}
          className="account-menu-positioner"
        >
          <Menu.Popup className="account-menu">
            <div className="account-menu-heading">
              Signed in as <b>{email}</b>
            </div>
            <Menu.Item render={<Link to="/account" />}>
              Account settings
            </Menu.Item>
            <Menu.Item render={<Link to="/accounts" />}>
              Linked accounts
            </Menu.Item>
            <Menu.Separator />
            <Menu.Item onClick={onLogout} disabled={busy}>
              Sign out
            </Menu.Item>
          </Menu.Popup>
        </Menu.Positioner>
      </Menu.Portal>
    </Menu.Root>
  )
}
