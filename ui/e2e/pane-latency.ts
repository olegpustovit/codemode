import { expect } from "@playwright/test"
import type { Page } from "@playwright/test"
import { writeFileSync } from "node:fs"

// Uses the real gateway fixture in gateway.spec.ts, including optimized builds.
export async function measurePaneLatency(page: Page, destination: string) {
  const client = await page.context().newCDPSession(page)
  await client.send("Performance.enable")
  const clicks: number[] = []
  const handlers: number[] = []
  const layouts: number[] = []
  const keys: number[] = []
  for (let sample = 0; sample < 15; sample++) {
    await page.goto("/integrations")
    await expect(
      page.getByRole("row", { name: "GitHub", exact: true })
    ).toBeVisible()
    const timing = await page.evaluate(
      () =>
        new Promise<{ total: number; handler: number; layout: number }>(
          (resolve, reject) => {
            const row = [
              ...document.querySelectorAll<HTMLElement>(".data-row"),
            ].find((r) => r.getAttribute("aria-label") === "GitHub")!
            const start = performance.now()
            row.click()
            const handler = performance.now() - start
            const painted = () => {
              const stack = document.querySelector<HTMLElement>(".pane-stack")!
              const detail = stack.querySelector<HTMLElement>(".detail-pane")!
              if (
                detail.getBoundingClientRect().width >=
                  stack.getBoundingClientRect().width * 0.53 &&
                detail.querySelector("h2")
              ) {
                const layout = performance.now() - start
                requestAnimationFrame(() =>
                  resolve({ total: performance.now() - start, handler, layout })
                )
              } else if (performance.now() - start > 2000)
                reject(new Error("Detail did not paint"))
              else requestAnimationFrame(painted)
            }
            requestAnimationFrame(painted)
          }
        )
    )
    clicks.push(timing.total)
    handlers.push(timing.handler)
    layouts.push(timing.layout)
    await page.getByRole("grid", { name: "Integrations", exact: true }).focus()
    const before = await client.send("Performance.getMetrics")
    await page.keyboard.press("ArrowDown")
    await page.evaluate(
      () =>
        new Promise<void>((done) =>
          requestAnimationFrame(() => requestAnimationFrame(() => done()))
        )
    )
    const after = await client.send("Performance.getMetrics")
    const script = (metrics: typeof before) =>
      metrics.metrics.find((m) => m.name === "ScriptDuration")!.value
    keys.push((script(after) - script(before)) * 1000)
  }
  await client.detach()
  await page.goto("/integrations")
  await page.locator(".backdrop canvas").waitFor({ state: "attached" })
  const p95 = (values: number[]) =>
    [...values].sort((a, b) => a - b)[Math.ceil(values.length * 0.95) - 1]
  const result = {
    binary: process.env.CODEMODE_GATEWAY_BIN,
    browser: page.context().browser()?.version(),
    viewport: page.viewportSize(),
    graphics: await page.evaluate(() => {
      const gl = document.querySelector("canvas")?.getContext("webgl")
      const debug = gl?.getExtension("WEBGL_debug_renderer_info")
      return {
        devicePixelRatio,
        webgl: !!gl,
        renderer: gl?.getParameter(
          debug?.UNMASKED_RENDERER_WEBGL ?? gl.RENDERER
        ),
      }
    }),
    samples: clicks.length,
    clickToPaintMs: { p95: p95(clicks), values: clicks },
    clickHandlerMs: { p95: p95(handlers), values: handlers },
    layoutReadyMs: { p95: p95(layouts), values: layouts },
    keyScriptMs: { p95: p95(keys), values: keys },
  }
  writeFileSync(destination, JSON.stringify(result, null, 2) + "\n")
  if (!process.env.CODEMODE_LATENCY_BASELINE) {
    expect(result.clickToPaintMs.p95).toBeLessThan(50)
    expect(result.keyScriptMs.p95).toBeLessThan(16)
  }
}

export async function measureLargeListKeys(page: Page, destination: string) {
  const client = await page.context().newCDPSession(page)
  await client.send("Performance.enable")
  const durations: number[] = []
  await page.keyboard.press("Home")
  for (let sample = 0; sample < 20; sample++) {
    const before = await client.send("Performance.getMetrics")
    await page.keyboard.press("ArrowDown")
    await page.evaluate(
      () =>
        new Promise<void>((done) =>
          requestAnimationFrame(() => requestAnimationFrame(() => done()))
        )
    )
    const after = await client.send("Performance.getMetrics")
    durations.push(
      (after.metrics.find((metric) => metric.name === "ScriptDuration")!.value -
        before.metrics.find((metric) => metric.name === "ScriptDuration")!
          .value) *
        1000
    )
  }
  await client.detach()
  const p95 = [...durations].sort((a, b) => a - b)[
    Math.ceil(durations.length * 0.95) - 1
  ]
  writeFileSync(
    destination,
    JSON.stringify(
      { rows: 300, samples: durations.length, p95, values: durations },
      null,
      2
    ) + "\n"
  )
  expect(p95).toBeLessThan(16)
}
