import { createContext, useContext } from "react"
import type { ReactNode } from "react"
import { createPortal } from "react-dom"
type Targets = {
  "shell-search": HTMLDivElement | null
  "status-keys": HTMLDivElement | null
}
export const ShellTargets = createContext<Targets>({
  "shell-search": null,
  "status-keys": null,
})
export function ShellSlot({
  target,
  children,
}: {
  target: keyof Targets
  children: ReactNode
}) {
  const host = useContext(ShellTargets)[target]
  return host ? createPortal(children, host) : null
}
