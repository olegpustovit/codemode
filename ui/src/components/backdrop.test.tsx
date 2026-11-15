import { afterEach, beforeEach, expect, test, vi } from "vitest"
import { cleanup, render } from "@testing-library/react"
import { Backdrop } from "./backdrop"

vi.mock("@tanstack/react-router", () => ({
  useRouterState: () => "/sources",
}))

let frames: Map<number, FrameRequestCallback>
let sequence: number
let reduced: boolean
let hidden: boolean
let motionChange: EventListener | undefined
const gl = {
  createProgram: vi.fn(() => ({})),
  createBuffer: vi.fn(() => ({})),
  createShader: vi.fn(() => ({})),
  shaderSource: vi.fn(),
  compileShader: vi.fn(),
  getShaderParameter: vi.fn(() => true),
  attachShader: vi.fn(),
  linkProgram: vi.fn(),
  getProgramParameter: vi.fn(() => true),
  useProgram: vi.fn(),
  bindBuffer: vi.fn(),
  bufferData: vi.fn(),
  getAttribLocation: vi.fn(() => 0),
  enableVertexAttribArray: vi.fn(),
  vertexAttribPointer: vi.fn(),
  getUniformLocation: vi.fn(() => ({})),
  uniform2f: vi.fn(),
  uniform1f: vi.fn(),
  uniform3f: vi.fn(),
  drawArrays: vi.fn(),
  viewport: vi.fn(),
  deleteShader: vi.fn(),
  deleteBuffer: vi.fn(),
  deleteProgram: vi.fn(),
}
function draw() {
  const pending = [...frames.values()]
  frames.clear()
  pending.forEach((frame) => frame(performance.now()))
}
beforeEach(() => {
  vi.clearAllMocks()
  frames = new Map()
  sequence = 0
  reduced = false
  hidden = false
  motionChange = undefined
  vi.stubGlobal("requestAnimationFrame", (frame: FrameRequestCallback) => {
    frames.set(++sequence, frame)
    return sequence
  })
  vi.stubGlobal("cancelAnimationFrame", (id: number) => frames.delete(id))
  vi.stubGlobal("matchMedia", () => ({
    get matches() {
      return reduced
    },
    addEventListener: (_: string, listener: EventListener) => {
      motionChange = listener
    },
    removeEventListener: vi.fn(),
  }))
  vi.spyOn(document, "hidden", "get").mockImplementation(() => hidden)
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockImplementation(
    () => gl as unknown as WebGLRenderingContext
  )
})
afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})
test("pauses hidden tabs, resumes once, caps resolution, and releases GPU resources", () => {
  vi.stubGlobal("devicePixelRatio", 3)
  const view = render(<Backdrop />)
  expect(view.container.querySelector("canvas")?.width).toBe(innerWidth * 2)
  draw()
  expect(gl.drawArrays).toHaveBeenCalledTimes(1)
  expect(frames.size).toBe(1)
  hidden = true
  document.dispatchEvent(new Event("visibilitychange"))
  expect(frames.size).toBe(0)
  hidden = false
  document.dispatchEvent(new Event("visibilitychange"))
  expect(frames.size).toBe(1)
  draw()
  expect(gl.drawArrays).toHaveBeenCalledTimes(2)
  view.unmount()
  expect(frames.size).toBe(0)
  expect(gl.deleteShader).toHaveBeenCalledTimes(2)
  expect(gl.deleteBuffer).toHaveBeenCalledTimes(1)
  expect(gl.deleteProgram).toHaveBeenCalledTimes(1)
})
test("reduced motion draws one static frame and responds to preference changes", () => {
  reduced = true
  render(<Backdrop />)
  draw()
  expect(gl.uniform1f).toHaveBeenLastCalledWith(expect.anything(), 20)
  expect(frames.size).toBe(0)
  reduced = false
  motionChange?.(new Event("change"))
  draw()
  expect(frames.size).toBe(1)
})
test("WebGL failure leaves the CSS fallback without scheduling animation", () => {
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null)
  const view = render(<Backdrop />)
  expect(view.container.querySelector(".backdrop")).not.toBeNull()
  expect(frames.size).toBe(0)
})
test("context loss stops rendering and reveals the fallback", () => {
  const view = render(<Backdrop />)
  const canvas = view.container.querySelector("canvas")!
  canvas.dispatchEvent(new Event("webglcontextlost"))
  expect(frames.size).toBe(0)
  expect(canvas.style.visibility).toBe("hidden")
})
