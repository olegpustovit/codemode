import { StrictMode, lazy, Suspense } from "react"
import { createRoot } from "react-dom/client"

import "./index.css"
import { QueryClientProvider } from "@tanstack/react-query"
import { RouterProvider } from "@tanstack/react-router"
import { TooltipProvider } from "@/components/ui/tooltip-provider"
import { queryClient, router } from "./router"
const Notifications = lazy(() =>
  import("@/components/ui/toast").then((module) => ({
    default: module.Toaster,
  }))
)

const root = document.getElementById("root")
if (!root) throw new Error("Application root is missing")
createRoot(root).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <TooltipProvider delay={0}>
        <RouterProvider router={router} />
        <Suspense fallback={null}>
          <Notifications />
        </Suspense>
      </TooltipProvider>
    </QueryClientProvider>
  </StrictMode>
)
