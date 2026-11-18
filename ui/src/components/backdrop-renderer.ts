import { fragmentShader } from "./backdrop-shader"

export type BackdropState = {
  hue: readonly number[]
  width: number
  height: number
  scale: number
  reduced: boolean
  visible: boolean
}

export function renderBackdrop(
  canvas: HTMLCanvasElement | OffscreenCanvas,
  initial: BackdropState,
  paintHue: (hue: string) => void,
  limitSoftware: boolean,
  softwareScheduler?: {
    request(callback: FrameRequestCallback): number
    cancel(id: number): void
  }
) {
  let state = initial
  const gl = canvas.getContext("webgl", {
    antialias: false,
    premultipliedAlpha: false,
  })
  if (!gl) return null
  const shaders: WebGLShader[] = []
  const program = gl.createProgram()
  const buffer = gl.createBuffer()
  let raf = 0
  let scheduler = {
    request: (callback: FrameRequestCallback) =>
      requestAnimationFrame(callback),
    cancel: (id: number) => cancelAnimationFrame(id),
  }
  const release = () => {
    scheduler.cancel(raf)
    shaders.forEach((shader) => gl.deleteShader(shader))
    gl.deleteBuffer(buffer)
    gl.deleteProgram(program)
  }
  const compile = (type: number, source: string) => {
    const shader = gl.createShader(type)
    if (!shader) return false
    shaders.push(shader)
    gl.shaderSource(shader, source)
    gl.compileShader(shader)
    if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS) || !program)
      return false
    gl.attachShader(program, shader)
    return true
  }
  if (
    !program ||
    !buffer ||
    !compile(
      gl.VERTEX_SHADER,
      "attribute vec2 p;void main(){gl_Position=vec4(p,0.,1.);}"
    ) ||
    !compile(gl.FRAGMENT_SHADER, fragmentShader)
  ) {
    release()
    return null
  }
  gl.linkProgram(program)
  if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
    release()
    return null
  }
  gl.useProgram(program)
  gl.bindBuffer(gl.ARRAY_BUFFER, buffer)
  gl.bufferData(
    gl.ARRAY_BUFFER,
    new Float32Array([-1, -1, 3, -1, -1, 3]),
    gl.STATIC_DRAW
  )
  const position = gl.getAttribLocation(program, "p")
  gl.enableVertexAttribArray(position)
  gl.vertexAttribPointer(position, 2, gl.FLOAT, false, 0, 0)
  const resolution = gl.getUniformLocation(program, "r")
  const time = gl.getUniformLocation(program, "t")
  const hue = gl.getUniformLocation(program, "h")
  const started = performance.now()
  const current = [...state.hue]
  let previous = started
  let paintedHue = ""
  const rendererInfo = gl.getExtension("WEBGL_debug_renderer_info")
  const renderer = rendererInfo
    ? String(gl.getParameter(rendererInfo.UNMASKED_RENDERER_WEBGL))
    : ""
  const software = /swiftshader|llvmpipe|software rasterizer/i.test(renderer)
  if (software && softwareScheduler) scheduler = softwareScheduler
  const frameInterval = 1000 / 24
  let paintedAt = -Infinity
  const frame = (now: number) => {
    raf = 0
    if (!state.visible) return
    if (software && !state.reduced && now - paintedAt < frameInterval) {
      raf = scheduler.request(frame)
      return
    }
    paintedAt = now
    const blend = state.reduced
      ? 1
      : 1 - Math.exp(-Math.min(now - previous, 100) / 270)
    previous = now
    for (let i = 0; i < 3; i++)
      current[i] += (state.hue[i] - current[i]) * blend
    const cssHue = current.map((v) => Math.round(v * 255)).join(",")
    if (cssHue !== paintedHue) {
      paintHue(cssHue)
      paintedHue = cssHue
    }
    gl.uniform2f(resolution, canvas.width, canvas.height)
    gl.uniform1f(time, state.reduced ? 20 : (now - started) / 1000)
    gl.uniform3f(hue, current[0], current[1], current[2])
    gl.drawArrays(gl.TRIANGLES, 0, 3)
    if (!state.reduced) raf = scheduler.request(frame)
  }
  const kick = () => {
    if (!raf && state.visible) raf = scheduler.request(frame)
  }
  const size = () => {
    const scale = software && limitSoftware ? 0.4 : state.scale
    canvas.width = Math.max(1, Math.round(state.width * scale))
    canvas.height = Math.max(1, Math.round(state.height * scale))
    gl.viewport(0, 0, canvas.width, canvas.height)
    paintedAt = -Infinity
    kick()
  }
  const visibility = () => {
    scheduler.cancel(raf)
    raf = 0
    previous = performance.now()
    paintedAt = -Infinity
    kick()
  }
  size()
  return {
    update(next: BackdropState) {
      const resized =
        next.width !== state.width ||
        next.height !== state.height ||
        next.scale !== state.scale
      const modeChanged =
        next.visible !== state.visible || next.reduced !== state.reduced
      state = next
      if (modeChanged) visibility()
      if (resized) size()
      kick()
    },
    dispose: release,
  }
}
