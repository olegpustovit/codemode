import { useEffect, useRef } from "react"
import { useRouterState } from "@tanstack/react-router"
import { renderBackdrop } from "./backdrop-renderer"
import type { BackdropState } from "./backdrop-renderer"
import type {
  BackdropWorkerReply,
  BackdropWorkerRequest,
} from "./backdrop.worker"

const hues = {
  sessions: [0.12, 0.26, 0.8],
  integrations: [0.36, 0.14, 0.74],
  workspaces: [0.04, 0.4, 0.42],
  scripts: [0.66, 0.32, 0.08],
  tokens: [0.62, 0.08, 0.22],
}

export function Backdrop() {
  const path = useRouterState({ select: (state) => state.location.pathname })
  const target = useRef(hues.sessions)
  const wake = useRef<(() => void) | null>(null)
  useEffect(() => {
    target.current = path.includes("scripts")
      ? hues.scripts
      : path.startsWith("/workspaces")
        ? hues.workspaces
        : path.startsWith("/tokens")
          ? hues.tokens
          : /^\/(sources|accounts|integrations)/.test(path)
            ? hues.integrations
            : hues.sessions
    if (wake.current) {
      wake.current()
    } else {
      document.documentElement.style.setProperty(
        "--hue",
        target.current.map((v) => Math.round(v * 255)).join(",")
      )
    }
  }, [path])
  const host = useRef<HTMLDivElement>(null)
  useEffect(() => {
    const element = host.current
    if (!element) return
    let canvas = document.createElement("canvas")
    element.append(canvas)
    const motion = matchMedia("(prefers-reduced-motion: reduce)")
    const state = (): BackdropState => ({
      hue: target.current,
      width: innerWidth,
      height: innerHeight,
      scale: Math.min(devicePixelRatio || 1, 2),
      reduced: motion.matches,
      visible: !document.hidden,
    })
    const paintHue = (hue: string) =>
      document.documentElement.style.setProperty("--hue", hue)
    let worker: Worker | undefined
    let admission: number | undefined
    const cancelAdmission = () => {
      if (admission !== undefined) cancelIdleCallback(admission)
      admission = undefined
    }
    const send = (
      message: BackdropWorkerRequest,
      transfer: Transferable[] = []
    ) => worker?.postMessage(message, transfer)
    const admitFrame = (frame: number) => {
      cancelAdmission()
      admission = requestIdleCallback((deadline) => {
        admission = undefined
        if (deadline.timeRemaining() < 8) admitFrame(frame)
        else send({ frame })
      })
    }
    let local: ReturnType<typeof renderBackdrop> = null
    const loseLocalContext = () => {
      local?.dispose()
      local = null
      wake.current = null
      canvas.style.visibility = "hidden"
      paintHue(target.current.map((value) => Math.round(value * 255)).join(","))
    }
    const startLocal = () => {
      local = renderBackdrop(canvas, state(), paintHue, true)
      if (!local) {
        loseLocalContext()
        return
      }
      canvas.addEventListener("webglcontextlost", loseLocalContext, {
        once: true,
      })
    }
    const fallback = () => {
      cancelAdmission()
      worker?.terminate()
      worker = undefined
      const replacement = document.createElement("canvas")
      canvas.replaceWith(replacement)
      canvas = replacement
      startLocal()
    }
    const sync = () => {
      const next = state()
      if (worker) send({ state: next })
      else local?.update(next)
    }
    wake.current = sync
    if (
      typeof canvas.transferControlToOffscreen === "function" &&
      typeof Worker !== "undefined"
    ) {
      try {
        worker = new Worker(new URL("./backdrop.worker.ts", import.meta.url), {
          type: "module",
        })
        worker.onmessage = ({ data }: MessageEvent<BackdropWorkerReply>) => {
          if (data === null) fallback()
          else if (typeof data === "string") paintHue(data)
          else admitFrame(data.frame)
        }
        worker.onerror = (event) => {
          event.preventDefault()
          fallback()
        }
        const initial = state()
        canvas.width = Math.max(1, Math.round(initial.width * initial.scale))
        canvas.height = Math.max(1, Math.round(initial.height * initial.scale))
        const offscreen = canvas.transferControlToOffscreen()
        send(
          {
            canvas: offscreen,
            state: initial,
            idleAdmission: typeof requestIdleCallback === "function",
          },
          [offscreen]
        )
      } catch {
        fallback()
      }
    } else {
      startLocal()
    }
    window.addEventListener("resize", sync)
    document.addEventListener("visibilitychange", sync)
    motion.addEventListener("change", sync)
    return () => {
      wake.current = null
      window.removeEventListener("resize", sync)
      document.removeEventListener("visibilitychange", sync)
      motion.removeEventListener("change", sync)
      canvas.removeEventListener("webglcontextlost", loseLocalContext)
      cancelAdmission()
      worker?.terminate()
      local?.dispose()
      canvas.remove()
    }
  }, [])
  return <div className="backdrop" aria-hidden="true" ref={host} />
}
