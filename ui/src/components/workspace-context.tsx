import { createContext, useContext } from "react"
export const WorkspaceContext = createContext<string | undefined>(undefined)
export function useWorkspace() {
  return useContext(WorkspaceContext)
}
