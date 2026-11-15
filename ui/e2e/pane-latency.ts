import { expect } from "@playwright/test"
import type { Page } from "@playwright/test"
import { writeFileSync } from "node:fs"

// Uses the real gateway fixture in gateway.spec.ts, including optimized builds.
export async function measurePaneLatency(page: Page, destination: string) {
  const client = await page.context().newCDPSession(page)
  await client.send("Performance.enable")
  const clicks: number[] = []
  const keys: number[] = []
  for (let sample = 0; sample < 15; sample++) {
    await page.goto("/integrations")
    await expect(
      page.getByRole("row", { name: "GitHub", exact: true })
    ).toBeVisible()
    clicks.push(
      await page.evaluate(
        () =>
          new Promise<number>((resolve, reject) => {
            const row = [
              ...document.querySelectorAll<HTMLElement>(".data-row"),
            ].find((r) => r.getAttribute("aria-label") === "GitHub")!
            const start = performance.now()
            row.click()
            const painted = () => {
              const stack = document.querySelector<HTMLElement>(".pane-stack")!
              const detail = stack.querySelector<HTMLElement>(".detail-pane")!
              if (
                detail.getBoundingClientRect().width >=
                  stack.getBoundingClientRect().width * 0.53 &&
                detail.querySelector("h2")
              )
                requestAnimationFrame(() => resolve(performance.now() - start))
              else if (performance.now() - start > 2000)
                reject(new Error("Detail did not paint"))
              else requestAnimationFrame(painted)
            }
            requestAnimationFrame(painted)
          })
      )
    )
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
  const p95 = (values: number[]) =>
    [...values].sort((a, b) => a - b)[Math.ceil(values.length * 0.95) - 1]
  const result = {
    binary: process.env.CODEMODE_GATEWAY_BIN,
    samples: clicks.length,
    clickToPaintMs: { p95: p95(clicks), values: clicks },
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
