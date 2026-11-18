import { renderBackdrop } from "./backdrop-renderer"
import type { BackdropState } from "./backdrop-renderer"

export type BackdropWorkerRequest =
  | { state: BackdropState; canvas?: OffscreenCanvas; idleAdmission?: boolean }
  | { frame: number }
export type BackdropWorkerReply = string | null | { frame: number }

let renderer: ReturnType<typeof renderBackdrop> = null
let pending: { id: number; callback: FrameRequestCallback } | undefined
const softwareScheduler = {
  request(callback: FrameRequestCallback) {
    const id = requestAnimationFrame(() => self.postMessage({ frame: id }))
    pending = { id, callback }
    return id
  },
  cancel(id: number) {
    cancelAnimationFrame(id)
    if (pending?.id === id) pending = undefined
  },
}

self.onmessage = ({ data }: MessageEvent<BackdropWorkerRequest>) => {
  if ("frame" in data) {
    if (pending?.id === data.frame) {
      const callback = pending.callback
      pending = undefined
      callback(performance.now())
    }
    return
  }
  const { canvas, state } = data
  if (canvas) {
    renderer = renderBackdrop(
      canvas,
      state,
      (hue) => self.postMessage(hue),
      false,
      data.idleAdmission ? softwareScheduler : undefined
    )
    if (!renderer) self.postMessage(null)
    else
      canvas.addEventListener(
        "webglcontextlost",
        () => self.postMessage(null),
        { once: true }
      )
  } else {
    renderer?.update(state)
  }
}
