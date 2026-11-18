import { useEffect, useRef } from "react"
import { useRouterState } from "@tanstack/react-router"
import { fragmentShader } from "./backdrop-shader"

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
  const wake = useRef<() => void>(() => {})
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
    document.documentElement.style.setProperty(
      "--hue",
      target.current.map((v) => Math.round(v * 255)).join(",")
    )
    wake.current()
  }, [path])
  const canvas = useRef<HTMLCanvasElement>(null)
  useEffect(() => {
    const cv = canvas.current
    if (!cv) return
    const gl = cv.getContext("webgl", {
      antialias: false,
      premultipliedAlpha: false,
    })
    if (!gl) return
    const shaders: WebGLShader[] = []
    const program = gl.createProgram()
    const buffer = gl.createBuffer()
    let raf = 0
    let lost = false
    const release = () => {
      cancelAnimationFrame(raf)
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
      return
    }
    gl.linkProgram(program)
    if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
      release()
      return
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
    const motion = matchMedia("(prefers-reduced-motion: reduce)")
    const started = performance.now()
    const current = [...target.current]
    let previous = started
    let paintedHue = ""
    const frame = (now: number) => {
      raf = 0
      if (document.hidden || lost) return
      const blend = motion.matches
        ? 1
        : 1 - Math.exp(-Math.min(now - previous, 100) / 270)
      previous = now
      for (let i = 0; i < 3; i++)
        current[i] += (target.current[i] - current[i]) * blend
      const cssHue = current.map((v) => Math.round(v * 255)).join(",")
      if (cssHue !== paintedHue) {
        document.documentElement.style.setProperty("--hue", cssHue)
        paintedHue = cssHue
      }
      gl.uniform2f(resolution, cv.width, cv.height)
      gl.uniform1f(time, motion.matches ? 20 : (now - started) / 1000)
      gl.uniform3f(hue, current[0], current[1], current[2])
      gl.drawArrays(gl.TRIANGLES, 0, 3)
      if (!motion.matches) raf = requestAnimationFrame(frame)
    }
    const kick = () => {
      if (!raf && !document.hidden && !lost) raf = requestAnimationFrame(frame)
    }
    const rendererInfo = gl.getExtension("WEBGL_debug_renderer_info")
    const renderer = rendererInfo
      ? String(gl.getParameter(rendererInfo.UNMASKED_RENDERER_WEBGL))
      : ""
    // A smaller backing buffer keeps the same scene moving on software WebGL.
    const software = /swiftshader|llvmpipe|software rasterizer/i.test(renderer)
    const size = () => {
      const scale = software ? 0.5 : Math.min(devicePixelRatio || 1, 2)
      cv.width = Math.max(1, Math.round(innerWidth * scale))
      cv.height = Math.max(1, Math.round(innerHeight * scale))
      gl.viewport(0, 0, cv.width, cv.height)
      kick()
    }
    const visibility = () => {
      cancelAnimationFrame(raf)
      raf = 0
      previous = performance.now()
      kick()
    }
    const contextLost = () => {
      lost = true
      cancelAnimationFrame(raf)
      raf = 0
      cv.style.visibility = "hidden"
    }
    wake.current = kick
    size()
    window.addEventListener("resize", size)
    document.addEventListener("visibilitychange", visibility)
    motion.addEventListener("change", visibility)
    cv.addEventListener("webglcontextlost", contextLost)
    return () => {
      wake.current = () => {}
      window.removeEventListener("resize", size)
      document.removeEventListener("visibilitychange", visibility)
      motion.removeEventListener("change", visibility)
      cv.removeEventListener("webglcontextlost", contextLost)
      release()
    }
  }, [])
  return (
    <div className="backdrop" aria-hidden="true">
      <canvas ref={canvas} />
    </div>
  )
}
