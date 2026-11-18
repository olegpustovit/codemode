import { renderBackdrop } from "./backdrop-renderer"
import type { BackdropState } from "./backdrop-renderer"

let renderer: ReturnType<typeof renderBackdrop> = null

self.onmessage = ({
  data,
}: MessageEvent<{ canvas?: OffscreenCanvas; state: BackdropState }>) => {
  const { canvas, state } = data
  if (canvas) {
    renderer = renderBackdrop(
      canvas,
      state,
      (hue) => self.postMessage(hue),
      false
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
