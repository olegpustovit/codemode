import { test as base, expect } from "@playwright/test"
import type { Source } from "../src/features/queries"
import type { Page, Locator } from "@playwright/test"
import { execFileSync, spawn } from "node:child_process"
import type { ChildProcess } from "node:child_process"
import { createServer, request as proxyRequest } from "node:http"
import { createServer as createTLS } from "node:https"
import { createHash, randomBytes } from "node:crypto"
import {
  mkdirSync,
  openSync,
  closeSync,
  writeFileSync,
  readFileSync,
} from "node:fs"
import { resolve } from "node:path"
import { once } from "node:events"
import { measurePaneLatency, measureLargeListKeys } from "./pane-latency"
import type axe from "axe-core"
declare global {
  interface Window {
    axe: typeof axe
  }
}

const root = resolve(import.meta.dirname, "../..")
const output = resolve(root, "target/ui-browser")
const hostname = "codemode.microapps.space"
const origin = `https://${hostname}`
const password = randomBytes(24).toString("hex")
const database = `codemode_browser_${process.pid}`
const maintenance =
  process.env.DATABASE_URL ??
  "postgresql://snowbear@localhost/postgres?host=/var/run/postgresql"
const databaseUrl = new URL(maintenance)
databaseUrl.pathname = `/${database}`
let server: ChildProcess | undefined
let loopback: string
let restartGateway: () => Promise<void>
let fixtureUrl: string
let tlsPort: number
let proxy: ReturnType<typeof createTLS> | undefined
const test = base.extend({
  browser: [
    async ({ playwright, launchOptions }, use) => {
      const browser = await playwright.chromium.launch({
        ...launchOptions,
        args: [
          ...(launchOptions.args ?? []),
          "--no-proxy-server",
          `--host-resolver-rules=MAP ${hostname} 127.0.0.1:${tlsPort},MAP client.example 127.0.0.1:${tlsPort}`,
        ],
      })
      await use(browser)
      await browser.close()
    },
    { scope: "worker" },
  ],
})
let destructiveCalls = 0
let readCalls = 0
let graphqlMutations = 0
const fixture = createServer(async (request, response) => {
  if (request.url === "/openapi.yaml" && request.method === "GET") {
    const server = fixtureUrl.replace(/\/mcp$/, "/rest")
    response.writeHead(200, { "Content-Type": "application/yaml" })
      .end(`openapi: 3.1.0
servers: [{url: ${server}}]
paths:
  /items:
    delete:
      operationId: deleteItems
      responses: {'200': {description: Deleted}}
    get:
      operationId: listItems
      responses:
        '200':
          content:
            application/json:
              schema: {type: array, items: {type: string}}
`)
    return
  }
  if (request.url === "/rest/items" && request.method === "DELETE") {
    if (request.headers["x-api-key"] !== "browser-api-key") {
      response.writeHead(401).end()
      return
    }
    destructiveCalls++
    response
      .writeHead(200, { "Content-Type": "application/json" })
      .end(JSON.stringify("deleted"))
    return
  }
  if (request.url === "/rest/items" && request.method === "GET") {
    readCalls++
    response
      .writeHead(
        request.headers["x-api-key"] === "browser-api-key" ? 200 : 401,
        { "Content-Type": "application/json" }
      )
      .end(JSON.stringify(["item-one"]))
    return
  }
  if (request.method !== "POST") {
    response.writeHead(405).end()
    return
  }
  const buffers: Buffer[] = []
  for await (const chunk of request) buffers.push(Buffer.from(chunk))
  const rpc = JSON.parse(Buffer.concat(buffers).toString())
  if (request.url === "/graphql") {
    if (request.headers.authorization !== "Bearer browser-graphql-secret") {
      response.writeHead(401).end()
      return
    }
    const mutation = rpc.query.trimStart().startsWith("mutation")
    if (mutation) graphqlMutations++
    response
      .writeHead(200, { "Content-Type": "application/graphql-response+json" })
      .end(
        JSON.stringify({
          data: {
            kind: mutation ? "mutation" : "query",
            variables: rpc.variables ?? null,
          },
        })
      )
    return
  }
  if (rpc.id === undefined) {
    response.writeHead(202).end()
    return
  }
  const result =
    rpc.method === "initialize"
      ? {
          protocolVersion: rpc.params.protocolVersion,
          capabilities: { tools: {} },
          serverInfo: { name: "browser-fixture", version: "1" },
        }
      : rpc.method === "tools/list"
        ? {
            tools:
              request.url === "/bulk-mcp"
                ? Array.from({ length: 300 }, (_, index) => ({
                    name: `${index < 150 ? "delete" : "get"}_item_${String(index).padStart(3, "0")}`,
                    description: "Bulk fixture tool",
                    inputSchema: { type: "object", properties: {} },
                    annotations: {
                      readOnlyHint: index >= 150,
                      destructiveHint: index < 150,
                    },
                  }))
                : [
                    {
                      name: "echo",
                      description: "Return a fixture value",
                      inputSchema: {
                        type: "object",
                        properties: { value: { type: "string" } },
                        required: ["value"],
                      },
                      annotations: { readOnlyHint: true },
                    },
                  ],
          }
        : {
            content: [
              {
                type: "text",
                text:
                  request.headers.authorization === "Bearer tenant-b-secret"
                    ? "second-tenant"
                    : request.headers.authorization ===
                        "Bearer browser-fixture-secret"
                      ? "first-tenant"
                      : "fixture",
              },
            ],
          }
  response
    .writeHead(200, { "Content-Type": "application/json" })
    .end(JSON.stringify({ jsonrpc: "2.0", id: rpc.id, result }))
})

test.beforeAll(async () => {
  execFileSync(
    process.execPath.endsWith("bun") ? process.execPath : "bun",
    ["run", "stage", "--check"],
    { cwd: resolve(root, "ui"), stdio: "pipe" }
  )
  mkdirSync(output, { recursive: true })
  execFileSync("createdb", ["--maintenance-db", maintenance, database])
  fixture.listen(0, "127.0.0.1")
  await once(fixture, "listening")
  const address = fixture.address()
  if (!address || typeof address === "string")
    throw new Error("Fixture did not bind")
  fixtureUrl = `http://127.0.0.1:${address.port}/mcp`
  const listener = createServer()
  listener.listen(0, "127.0.0.1")
  await once(listener, "listening")
  const available = listener.address()
  if (!available || typeof available === "string")
    throw new Error("No loopback port")
  const port = available.port
  await new Promise<void>((done) => listener.close(() => done()))
  loopback = `http://127.0.0.1:${port}`
  const sources = resolve(output, "sources.toml")
  writeFileSync(sources, "sources = []\n", { mode: 0o600 })
  const binary =
    process.env.CODEMODE_GATEWAY_BIN ?? resolve(root, "target/debug/codemode")
  const env = {
    ...process.env,
    CODEMODE_DATABASE_URL: databaseUrl.href,
    CODEMODE_HOSTNAME: hostname,
    CODEMODE_BIND: `127.0.0.1:${port}`,
    CODEMODE_SOURCES_FILE: sources,
    CODEMODE_MONTY_BIN: resolve(root, ".tools/monty/bin/monty"),
    CODEMODE_ENCRYPTION_KEY: randomBytes(32).toString("hex"),
    CODEMODE_LOG: "warn",
  }
  restartGateway = async () => {
    if (server && server.exitCode === null) {
      server.kill("SIGTERM")
      await once(server, "exit")
    }
    const log = openSync(resolve(output, "gateway.log"), "a", 0o600)
    server = spawn(binary, [], {
      env,
      stdio: ["ignore", log, log],
    })
    closeSync(log)
    await expect
      .poll(
        async () => {
          if (server?.exitCode !== null)
            throw new Error(
              "Gateway exited; inspect target/ui-browser/gateway.log"
            )
          return fetch(`${loopback}/healthz`, {
            signal: AbortSignal.timeout(500),
          })
            .then((response) => response.ok)
            .catch(() => false)
        },
        { timeout: 10_000 }
      )
      .toBe(true)
  }
  await restartGateway()
  execFileSync(
    "python3",
    [resolve(root, "tools/smoke-ui.py"), loopback, hostname],
    { stdio: "pipe" }
  )
  const key = resolve(output, "tls.key")
  const cert = resolve(output, "tls.crt")
  execFileSync(
    "openssl",
    [
      "req",
      "-x509",
      "-newkey",
      "rsa:2048",
      "-nodes",
      "-days",
      "1",
      "-subj",
      `/CN=${hostname}`,
      "-addext",
      `subjectAltName=DNS:${hostname},DNS:client.example`,
      "-keyout",
      key,
      "-out",
      cert,
    ],
    { stdio: "pipe" }
  )
  proxy = createTLS(
    { key: readFileSync(key), cert: readFileSync(cert) },
    (request, response) => {
      if (request.headers.host === "client.example") {
        response
          .writeHead(200, { "Content-Type": "text/html" })
          .end("<p>Returned to test client</p>")
        return
      }
      const upstream = proxyRequest(
        `${loopback}${request.url}`,
        { method: request.method, headers: request.headers },
        (result) => {
          response.writeHead(result.statusCode ?? 502, result.headers)
          result.pipe(response)
        }
      )
      upstream.on("error", () =>
        response.writeHead(502).end("Local gateway unavailable")
      )
      request.pipe(upstream)
    }
  )
  proxy.listen(0, "127.0.0.1")
  await once(proxy, "listening")
  const tls = proxy.address()
  if (!tls || typeof tls === "string") throw new Error("TLS proxy did not bind")
  tlsPort = tls.port
})

test.afterAll(async () => {
  if (proxy?.listening) {
    proxy.closeAllConnections()
    await new Promise<void>((done) => proxy!.close(() => done()))
  }
  if (server && server.exitCode === null) {
    server.kill("SIGTERM")
    await once(server, "exit")
  }
  if (fixture.listening)
    await new Promise<void>((done) => fixture.close(() => done()))
  execFileSync("dropdb", [
    "--if-exists",
    "--force",
    "--maintenance-db",
    maintenance,
    database,
  ])
})

async function login(page: Page, path = "/sources") {
  await page.goto(path)
  await expect(
    page.getByRole("heading", {
      name: /^(Create your account|Sign in to your control plane)$/,
    })
  ).toBeVisible()
  const creating = await page
    .getByRole("heading", { name: "Create your account", exact: true })
    .isVisible()
  await page.getByLabel("Email", { exact: true }).fill("ursus95@gmail.com")
  await page.getByLabel("Password", { exact: true }).fill(password)
  await page
    .getByRole("button", {
      name: creating ? "Create account" : "Sign in",
      exact: true,
    })
    .click()
  await expect(page).toHaveURL(
    `${origin}${path === "/sources" ? "/integrations" : path}`
  )
}
async function capture(page: Page, file: string) {
  await page.evaluate(() => document.fonts.ready.then(() => undefined))
  await page.evaluate(() => scrollTo(0, 0))
  await page.evaluate(
    () =>
      new Promise<void>((done) =>
        requestAnimationFrame(() => requestAnimationFrame(() => done()))
      )
  )
  await page.screenshot({
    path: resolve(output, file),
    fullPage: true,
    animations: "disabled",
    mask: [page.getByLabel("New access token")],
  })
}
async function accessibility(page: Page, file: string) {
  await page.evaluate(
    readFileSync(resolve(root, "ui/node_modules/axe-core/axe.min.js"), "utf8")
  )
  const result = await page.evaluate(() =>
    window.axe.run(document, {
      runOnly: { type: "tag", values: ["wcag2a", "wcag2aa", "wcag21aa"] },
    })
  )
  writeFileSync(resolve(output, file), JSON.stringify(result, null, 2))
  expect(
    result.violations.map((violation) => ({
      id: violation.id,
      nodes: violation.nodes.map((node) => node.target),
    }))
  ).toEqual([])
}
async function noOverflow(page: Page) {
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth
    )
  ).toBe(true)
}
async function contrast(button: Locator) {
  const pixels = (await button.screenshot()).toString("base64")
  return button.evaluate(async (element, pixels) => {
    const image = new Image()
    image.src = `data:image/png;base64,${pixels}`
    await image.decode()
    const canvas = document.createElement("canvas")
    canvas.width = canvas.height = 1
    const context = canvas.getContext("2d")
    if (!context) throw new Error("Canvas unavailable for contrast measurement")
    const style = getComputedStyle(element)
    context.drawImage(image, 3, 3, 1, 1, 0, 0, 1, 1)
    const background = Array.from(context.getImageData(0, 0, 1, 1).data).slice(
      0,
      3
    )
    context.fillStyle = style.color
    context.fillRect(0, 0, 1, 1)
    const foreground = Array.from(context.getImageData(0, 0, 1, 1).data).slice(
      0,
      3
    )
    const luminance = (rgb: number[]) =>
      rgb
        .map((value) => value / 255)
        .map((value) =>
          value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4
        )
        .reduce(
          (sum, value, index) => sum + value * [0.2126, 0.7152, 0.0722][index],
          0
        )
    const a = luminance(background),
      b = luminance(foreground)
    return {
      foreground,
      background,
      ratio: (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05),
    }
  }, pixels)
}
async function destructiveContrast(button: Locator, file: string) {
  const normal = await contrast(button)
  expect(normal.ratio).toBeGreaterThanOrEqual(4.5)
  await button.hover()
  await button.evaluate(async (element) => {
    void getComputedStyle(element).backgroundColor
    await Promise.all(
      element.getAnimations().map((animation) => animation.finished)
    )
  })
  const hover = await contrast(button)
  expect(hover.ratio).toBeGreaterThanOrEqual(4.5)
  writeFileSync(
    resolve(output, file),
    JSON.stringify({ normal, hover }, null, 2)
  )
}

test("desktop: real sources, endpoint scope, one-time token, credentials, consent and logout", async ({
  page,
}) => {
  const errors: string[] = []
  page.on("pageerror", (error) => errors.push(error.message))
  await page.goto("/tokens")
  await expect(
    page.getByRole("heading", { name: "Create your account" })
  ).toBeVisible()
  await noOverflow(page)
  await capture(page, "registration-desktop.png")
  await page.setViewportSize({ width: 390, height: 844 })
  await noOverflow(page)
  await capture(page, "registration-mobile.png")
  await page.setViewportSize({ width: 1440, height: 1000 })
  await page.getByLabel("Email", { exact: true }).fill("ursus95@gmail.com")
  await page.getByLabel("Password", { exact: true }).fill(password)
  await page
    .getByRole("button", { name: "Create account", exact: true })
    .click()
  await expect(page).toHaveURL(`${origin}/tokens`)
  expect(
    await page.evaluate(() =>
      fetch("/api/session")
        .then((r) => r.json())
        .then((s) => s.user.admin)
    )
  ).toBe(true)
  await page.getByRole("button", { name: /^Account menu for/ }).click()
  await page.getByRole("menuitem", { name: "Sign out", exact: true }).click()
  await page.goto("/tokens")
  await page.getByLabel("Email", { exact: true }).fill("ursus95@gmail.com")
  await page
    .getByLabel("Password", { exact: true })
    .fill("wrong-browser-password")
  await page.getByRole("button", { name: "Sign in", exact: true }).click()
  await expect(page.getByRole("alert")).toContainText(
    "incorrect email or password"
  )
  await page.getByLabel("Email", { exact: true }).fill("ursus95@gmail.com")
  await page.getByLabel("Password", { exact: true }).fill(password)
  await page.getByRole("button", { name: "Sign in", exact: true }).click()
  await expect(page).toHaveURL(`${origin}/tokens`)
  await page.goto("/sources")
  await expect(
    page.getByRole("heading", { name: "Integrations", exact: true })
  ).toBeVisible()
  await capture(page, "desktop-empty.png")
  await page
    .getByRole("link", { name: "Add by URL", exact: true })
    .first()
    .click()
  await page.getByLabel("Name", { exact: true }).fill("browser-fixture")
  await page.getByLabel("Server address").fill(fixtureUrl)
  await page.getByRole("button", { name: "Add source", exact: true }).click()
  await expect(page).toHaveURL(/\/sources\/[0-9a-f-]+$/)
  const sourceId = new URL(page.url()).pathname.split("/").at(-1)!
  await page.goto("/workspaces/new")
  await page.getByLabel("Workspace name").fill("browser-work")
  await expect(
    page.getByText("No sources selected. Agents will see no tools.")
  ).toBeVisible()
  await page
    .getByRole("button", { name: "Create workspace", exact: true })
    .click()
  await expect(page).toHaveURL(/\/workspaces\/[0-9a-f-]+$/)
  const workspaceId = new URL(page.url()).pathname.split("/").at(-1)!
  await page.reload()
  await expect(
    page.getByText("No sources selected. Agents will see no tools.")
  ).toBeVisible()
  await page.getByRole("checkbox", { name: /browser-fixture/ }).check()
  await page.getByRole("button", { name: "Save workspace" }).click()
  await page.reload()
  await expect(
    page.getByRole("checkbox", { name: /browser-fixture/ })
  ).toBeChecked()
  await destructiveContrast(
    page.getByRole("button", { name: "Delete workspace", exact: true }),
    "endpoint-contrast.json"
  )
  await page.mouse.move(500, 600)
  await capture(page, "desktop-endpoint.png")
  await page.setViewportSize({ width: 390, height: 844 })
  await noOverflow(page)
  await capture(page, "workspace-editor-mobile.png")
  await page.setViewportSize({ width: 1440, height: 1000 })
  await page.goto("/tokens")
  await page
    .locator(".list-heading")
    .getByRole("button", { name: "New personal token", exact: true })
    .click()
  await page.getByLabel("Name", { exact: true }).fill("Browser agent")
  await page.getByLabel("Workspace", { exact: true }).selectOption(workspaceId)
  await page.getByRole("button", { name: "Create token", exact: true }).click()
  const token = await page.getByLabel("New access token").inputValue()
  expect(token).toMatch(/^cm_/)
  await expect(
    page
      .locator(".code-panel")
      .filter({ hasText: "codex mcp add codemode --url" })
  ).toContainText(
    "https://codemode.microapps.space/mcp --bearer-token-env-var CODEMODE_TOKEN"
  )
  await expect(
    page
      .locator(".code-panel")
      .filter({ hasText: "default_tools_approval_mode" })
  ).toContainText('omit_tools_from = ["deferred"]')
  await expect(
    page.getByText("code_mode_tool_search = false", { exact: false })
  ).toBeVisible()
  await capture(page, "token-setup-desktop.png")
  await page.setViewportSize({ width: 390, height: 844 })
  await noOverflow(page)
  await capture(page, "token-setup-mobile.png")
  await page.setViewportSize({ width: 1440, height: 1000 })
  await page.getByRole("button", { name: "I saved it" }).click()
  await expect(page.getByLabel("New access token")).toHaveCount(0)
  await page.reload()
  await expect(page.getByLabel("New access token")).toHaveCount(0)
  await expect(
    page
      .getByRole("grid", { name: "Agents with access", exact: true })
      .getByRole("row", { name: "Browser agent", exact: true })
  ).toBeVisible()
  await expect
    .poll(() =>
      page.evaluate(
        (id) =>
          fetch(`/api/sources/${id}`)
            .then((response) => response.json())
            .then((source) => source.status),
        sourceId
      )
    )
    .toBe("ready")
  const source = await page.evaluate(
    (id) => fetch(`/api/sources/${id}`).then((response) => response.json()),
    sourceId
  )
  const code = `print("browser execution")\nreturn await tools.${source.pythonName}.echo(value="fixture-value")`
  const call = await page.request.post(`${loopback}/mcp`, {
    headers: {
      host: hostname,
      Authorization: `Bearer ${token}`,
      Accept: "application/json, text/event-stream",
      "MCP-Protocol-Version": "2025-11-25",
    },
    data: {
      jsonrpc: "2.0",
      id: 1,
      method: "tools/call",
      params: { name: "execute", arguments: { code } },
    },
  })
  expect(call.ok()).toBe(true)
  const rpc = await call.json()
  expect(rpc.result.isError).not.toBe(true)
  const execution = JSON.parse(rpc.result.content[0].text)
  expect(execution.status).toBe("completed")
  expect(execution.toolCalls).toBe(1)
  const history = await page.evaluate(() =>
    fetch("/api/sessions").then((r) => r.json())
  )
  const session = history.sessions.find(
    (entry: { workspaceId: string; client: string }) =>
      entry.workspaceId === workspaceId && entry.client === "Browser agent"
  )
  expect(session.title).toBe(source.pythonName)
  expect(
    new Date(session.expiresAt).getTime() -
      new Date(session.lastSeenAt).getTime()
  ).toBe(4 * 24 * 3600 * 1000)
  const stored = await page.evaluate(
    (id) => fetch(`/api/sessions/${id}`).then((r) => r.json()),
    session.id
  )
  expect(stored.executions[0].result).toBe(JSON.stringify(execution.result))
  expect(stored.executions[0].output).toBe("browser execution\n")
  expect(stored.executions[0].calls[0].preview).toBe(
    JSON.stringify(execution.result)
  )
  expect(
    await page.evaluate(
      (id) =>
        fetch(`/api/sessions/${id}`, {
          method: "PATCH",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ title: "Browser walkthrough" }),
        }).then((r) => r.status),
      session.id
    )
  ).toBe(204)
  const renamed = await page.evaluate(() =>
    fetch("/api/sessions?q=Browser%20walkthrough").then((r) => r.json())
  )
  expect(renamed.sessions[0].title).toBe("Browser walkthrough")
  await page.goto("/sources")
  await expect(
    page.getByRole("row", { name: "browser-fixture", exact: true })
  ).toBeVisible()
  await page.evaluate(() => document.fonts.ready)
  const cdp = await page.context().newCDPSession(page)
  await cdp.send("DOM.enable")
  await cdp.send("CSS.enable")
  const documentNode = await cdp.send("DOM.getDocument")
  const headingNode = await cdp.send("DOM.querySelector", {
    nodeId: documentNode.root.nodeId,
    selector: "h1",
  })
  const fonts = await cdp.send("CSS.getPlatformFontsForNode", {
    nodeId: headingNode.nodeId,
  })
  writeFileSync(
    resolve(output, "rendered-fonts.json"),
    JSON.stringify(fonts, null, 2)
  )
  expect(
    fonts.fonts.some(
      (font) => font.isCustomFont && font.familyName.includes("M PLUS 1p")
    )
  ).toBe(true)
  await cdp.detach()
  await capture(page, "desktop-sources.png")
  await page.goto(`/executions/${execution.executionId}`)
  await expect(page).toHaveURL(/\/sessions\?.*execution=/)
  await expect(page.getByLabel("Python code", { exact: true })).toBeVisible()
  await page
    .getByRole("button", {
      name: `tools.${source.pythonName}.echo`,
      exact: true,
    })
    .click()
  await expect(
    page.locator(".trace-kv").filter({ hasText: "fixture-value" })
  ).toBeVisible()
  await page.reload()
  await expect(page.getByLabel("Python code", { exact: true })).toBeVisible()
  await expect(
    page
      .getByRole("heading", { name: "Printed", exact: true })
      .locator("..")
      .locator("pre")
  ).toHaveText("browser execution\n")
  await expect(page.locator(".trace-return dd")).toHaveText(
    JSON.stringify(execution.result)
  )
  await capture(page, "desktop-execution.png")
  await page.goto(`/sources/${sourceId}/accounts`)
  await page
    .getByLabel("Bearer token", { exact: true })
    .fill("browser-fixture-secret")
  await page.getByRole("button", { name: "Save credential" }).click()
  await expect(
    page.getByText("Bearer token linked", { exact: true })
  ).toBeVisible()
  await expect(
    page.getByLabel("Replace bearer token", { exact: true })
  ).toHaveValue("")
  expect(
    await page.evaluate(() =>
      fetch("/api/accounts").then((response) => response.text())
    )
  ).not.toContain("browser-fixture-secret")
  const registration = await page.evaluate(() =>
    fetch("/oauth/register", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        client_name: "Browser test client",
        redirect_uris: ["https://client.example/callback"],
        token_endpoint_auth_method: "none",
      }),
    }).then((response) => response.json())
  )
  expect(registration.client_id).toEqual(expect.any(String))
  const challenge = createHash("sha256")
    .update("browser-verifier-123456789012345678901234567890123456789")
    .digest("base64url")
  const query = new URLSearchParams({
    response_type: "code",
    client_id: registration.client_id,
    redirect_uri: "https://client.example/callback",
    resource: `${origin}/mcp`,
    code_challenge: challenge,
    code_challenge_method: "S256",
    state: "browser-state",
    scope: "mcp",
  })
  await page.goto(`/oauth/authorize?${query}`)
  await expect(
    page.getByRole("heading", { name: "Authorize Browser test client?" })
  ).toBeVisible()
  await expect(
    page.getByRole("button", { name: "Authorize workspace", exact: true })
  ).toBeDisabled()
  await page.getByLabel("Workspace", { exact: true }).selectOption(workspaceId)
  await expect(page.getByRole("alert")).toContainText("browser-fixture")
  await capture(page, "desktop-consent.png")
  await page.setViewportSize({ width: 390, height: 844 })
  await noOverflow(page)
  await capture(page, "workspace-consent-mobile.png")
  await page.setViewportSize({ width: 1440, height: 1000 })
  await page.getByRole("button", { name: "Deny", exact: true }).click()
  await expect(page).toHaveURL(
    /client\.example\/callback\?.*error=access_denied/
  )
  expect(new URL(page.url()).searchParams.get("state")).toBe("browser-state")
  await page.goto(`${origin}/oauth/authorize?${query}`)
  await page.getByLabel("Workspace", { exact: true }).selectOption(workspaceId)
  await page
    .getByRole("button", { name: "Authorize workspace", exact: true })
    .click()
  await expect(page).toHaveURL(/client\.example\/callback\?.*code=/)
  expect(new URL(page.url()).searchParams.get("iss")).toBe(origin)
  await page.goto(`${origin}/workspaces/${workspaceId}`)
  await page
    .getByRole("button", { name: "Delete workspace", exact: true })
    .click()
  await expect(page.getByRole("alertdialog")).toHaveCount(0)
  await expect(page.getByRole("status")).toContainText("within 3 seconds")
  await page
    .getByRole("button", { name: "Delete workspace", exact: true })
    .click()
  await expect(page).toHaveURL(`${origin}/workspaces`)
  const revoked = await page.request.get(`${loopback}/mcp`, {
    headers: {
      host: hostname,
      Authorization: `Bearer ${token}`,
    },
  })
  expect(revoked.status()).toBe(401)
  await page.getByRole("button", { name: /^Account menu for/ }).click()
  await page.getByRole("menuitem", { name: "Sign out", exact: true }).click()
  await expect(
    page.getByRole("heading", { name: "Sign in to your control plane" })
  ).toBeVisible()
  await page.goBack()
  await expect(
    page.getByRole("heading", { name: "Sign in to your control plane" })
  ).toBeVisible()
  await noOverflow(page)
  expect(errors).toEqual([])
})

test("mobile: navigation, filter URL, deep-link refresh and narrow content", async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 844 })
  await login(page)
  await expect(
    page.getByRole("heading", { name: "Integrations", exact: true })
  ).toBeVisible()
  await capture(page, "mobile-sources.png")
  await noOverflow(page)
  await expect(page.getByRole("navigation", { name: "Sections" })).toBeVisible()
  await capture(page, "mobile-navigation.png")
  await page.getByRole("link", { name: "Workspaces", exact: true }).click()
  await expect(
    page.getByRole("heading", { name: "All sources", exact: true })
  ).toBeVisible()
  await expect(page.getByRole("navigation", { name: "Sections" })).toBeVisible()
  await page.goto("/sources?q=browser")
  await page.reload()
  await expect(page.getByLabel("Search this list")).toHaveValue("browser")
  await page.goto("/sources/new")
  await page.getByLabel("Name", { exact: true }).fill("x".repeat(64))
  await page.getByLabel("Server address").fill(fixtureUrl)
  await page.getByRole("button", { name: "Add source", exact: true }).click()
  await expect(page).toHaveURL(/\/sources\/[0-9a-f-]+$/)
  await expect(
    page.getByRole("heading", { name: "x".repeat(64), exact: true })
  ).toBeVisible()
  await destructiveContrast(
    page.getByRole("button", { name: "Remove source", exact: true }),
    "source-contrast.json"
  )
  await page.mouse.move(10, 10)
  await capture(page, "mobile-source-detail.png")
  await noOverflow(page)
})

test("OpenAPI: add YAML document, pin the server, link a header key and execute through Monty", async ({
  page,
}) => {
  await login(page, "/sources/new")
  await page.getByLabel("Name", { exact: true }).fill("browser-openapi")
  await page.getByLabel("Connection type").selectOption("openapi")
  await page
    .getByLabel("OpenAPI document address")
    .fill(fixtureUrl.replace(/\/mcp$/, "/openapi.yaml"))
  await page
    .getByRole("checkbox", { name: /Require a linked account/i })
    .check()
  await capture(page, "openapi-new-source.png")
  await page.getByRole("button", { name: "Add source", exact: true }).click()
  await expect(page).toHaveURL(/\/sources\/[0-9a-f-]+$/)
  const sourceId = new URL(page.url()).pathname.split("/").at(-1)!
  await expect(page.getByLabel("API server address")).toHaveValue(
    fixtureUrl.replace(/\/mcp$/, "/rest")
  )
  await page.reload()
  await expect(page.getByLabel("API server address")).toHaveValue(
    fixtureUrl.replace(/\/mcp$/, "/rest")
  )
  await page.getByRole("link", { name: "Manage account", exact: true }).click()
  await expect(
    page.getByRole("button", { name: "Connect with OAuth" })
  ).toHaveCount(0)
  await page.getByLabel("Credential type").selectOption("api_key_header")
  await page.getByLabel("Header name", { exact: true }).fill("X-Api-Key")
  await page.getByLabel("API key", { exact: true }).fill("browser-api-key")
  await capture(page, "openapi-account.png")
  await page
    .getByRole("button", { name: "Save credential", exact: true })
    .click()
  await expect(page.getByText("HTTP credential linked")).toBeVisible()
  await expect(page.getByLabel("API key", { exact: true })).toHaveValue("")
  await page.reload()
  await expect(page.getByText("HTTP credential linked")).toBeVisible()
  await expect(page.getByLabel("Bearer token", { exact: true })).toHaveValue("")
  await expect
    .poll(() =>
      page.evaluate(
        (id) =>
          fetch(`/api/sources/${id}`)
            .then((r) => r.json())
            .then((s) => s.status),
        sourceId
      )
    )
    .toBe("ready")
  const issued = await page.evaluate(async () => {
    const response = await fetch("/api/tokens", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        name: "OpenAPI browser check",
        workspaceId: (
          await fetch("/api/workspaces").then((r) => r.json())
        ).find((w: { allSources: boolean }) => w.allSources).id,
      }),
    })
    if (!response.ok) throw new Error("Could not issue fixture token")
    return response.json()
  })
  try {
    const source = await page.evaluate(
      (id) => fetch(`/api/sources/${id}`).then((r) => r.json()),
      sourceId
    )
    const response = await page.request.post(`${loopback}/mcp`, {
      headers: {
        host: hostname,
        Authorization: `Bearer ${issued.token}`,
        Accept: "application/json, text/event-stream",
        "MCP-Protocol-Version": "2025-11-25",
      },
      data: {
        jsonrpc: "2.0",
        id: 1,
        method: "tools/call",
        params: {
          name: "execute",
          arguments: {
            code: `return await tools.${source.pythonName}.listItems()`,
          },
        },
      },
    })
    expect(response.ok()).toBe(true)
    const rpc = await response.json()
    expect(rpc.result.isError).not.toBe(true)
    const execution = JSON.parse(rpc.result.content[0].text)
    expect(execution.status).toBe("completed")
    expect(execution.result).toEqual(["item-one"])
    await page.goto(`/executions/${execution.executionId}`)
    await expect(
      page
        .locator("summary")
        .filter({ hasText: `${source.pythonName}.listItems` })
    ).toBeVisible()
    const invoke = async (name: string, args: unknown) => {
      const response = await page.request.post(`${loopback}/mcp`, {
        headers: {
          host: hostname,
          Authorization: `Bearer ${issued.token}`,
          Accept: "application/json, text/event-stream",
          "MCP-Protocol-Version": "2025-11-25",
        },
        data: {
          jsonrpc: "2.0",
          id: 2,
          method: "tools/call",
          params: { name, arguments: args },
        },
      })
      expect(response.ok()).toBe(true)
      return (await response.json()).result
    }
    const workspaces = await page.evaluate(() =>
      fetch("/api/workspaces").then((r) => r.json())
    )
    const main = workspaces.find((e: { allSources: boolean }) => e.allSources)
    await page.goto(`/workspaces/${main.id}`)
    await page.getByRole("link", { name: "Manage tool policies" }).click()
    await expect(
      page.getByRole("heading", { name: "All sources", exact: true })
    ).toBeVisible()
    const update = async (path: string, action: () => Promise<unknown>) => {
      const response = page.waitForResponse(
        (response) =>
          new URL(response.url()).pathname === path &&
          response.request().method() === "PUT"
      )
      await action()
      expect((await response).status()).toBe(204)
    }
    const readRow = page
      .getByRole("grid", { name: "Workspace tools", exact: true })
      .getByRole("row", { name: `${source.pythonName}.listItems`, exact: true })
    const readSwitch = readRow.getByRole("switch")
    await update(`/api/workspaces/${main.id}/policies`, () =>
      readSwitch.click()
    )
    await expect(readSwitch).toHaveAttribute("aria-checked", "false")
    await page.reload()
    await expect(readSwitch).toHaveAttribute("aria-checked", "false")
    const blocked = await invoke("search", { query: "listItems" })
    const blockedTools = JSON.parse(blocked.content[0].text).tools
    expect(blockedTools).toHaveLength(1)
    expect(blockedTools[0].blocked).toBe(true)
    await capture(page, "tool-policies.png")
    await readRow.click()
    const reset = page
      .getByRole("region", { name: "Workspace tool details", exact: true })
      .getByRole("button", { name: "Use default", exact: true })
    await update(`/api/workspaces/${main.id}/policies`, () => reset.click())
    await expect(reset).toBeDisabled()
    await page
      .getByLabel("Tool scope", { exact: true })
      .selectOption("selected")
    await update(`/api/workspaces/${main.id}/scope`, () =>
      page.getByRole("button", { name: "Save tool scope", exact: true }).click()
    )
    await expect(
      page.getByRole("button", { name: "Save tool scope", exact: true })
    ).toHaveCount(0)
    await page.reload()
    await expect(page.getByLabel("Tool scope", { exact: true })).toHaveValue(
      "selected"
    )
    expect(
      JSON.parse(
        (await invoke("search", { query: "listItems" })).content[0].text
      ).tools
    ).toHaveLength(0)
    const readScope = page.getByRole("checkbox", {
      name: `Include ${source.pythonName}.listItems`,
      exact: true,
    })
    await update(`/api/workspaces/${main.id}/scope/tools`, () =>
      readScope.check()
    )
    await expect(readScope).toBeEnabled()
    await expect(readScope).toBeChecked()
    expect(
      JSON.parse(
        (await invoke("search", { query: "listItems" })).content[0].text
      ).tools
    ).toHaveLength(1)
    const writeScope = page.getByRole("checkbox", {
      name: `Include ${source.pythonName}.deleteItems`,
      exact: true,
    })
    await update(`/api/workspaces/${main.id}/scope/tools`, () =>
      writeScope.check()
    )
    await expect(writeScope).toBeEnabled()
    await expect(writeScope).toBeChecked()
    await capture(page, "tool-scope-desktop.png")
    await page.setViewportSize({ width: 390, height: 844 })
    await noOverflow(page)
    await capture(page, "tool-scope-mobile.png")
    await page.setViewportSize({ width: 1440, height: 1000 })
    const readsBefore = readCalls
    const completed = JSON.parse(
      (
        await invoke("execute", {
          code: `print("calling allowed tools")\nreturn await asyncio.gather(tools.${source.pythonName}.listItems(), tools.${source.pythonName}.deleteItems())`,
        })
      ).content[0].text
    )
    expect(completed.status).toBe("completed")
    expect(completed.result).toEqual([["item-one"], "deleted"])
    expect(completed.output).toBe("calling allowed tools\n")
    expect(destructiveCalls).toBe(1)
    expect(readCalls).toBe(readsBefore + 1)
    const writeSwitch = page
      .getByRole("grid", { name: "Workspace tools", exact: true })
      .getByRole("row", {
        name: `${source.pythonName}.deleteItems`,
        exact: true,
      })
      .getByRole("switch")
    await update(`/api/workspaces/${main.id}/policies`, () =>
      writeSwitch.click()
    )
    const blockedWrite = JSON.parse(
      (
        await invoke("execute", {
          code: `try:\n    await tools.${source.pythonName}.deleteItems()\nexcept PermissionError as e:\n    return str(e)`,
        })
      ).content[0].text
    )
    expect(blockedWrite.status).toBe("completed")
    expect(blockedWrite.result).toContain(`${source.pythonName}.deleteItems`)
    expect(blockedWrite.result).toContain(main.id)
    expect(destructiveCalls).toBe(1)
    expect(
      await page.evaluate(() =>
        fetch("/api/approvals").then((response) => response.status)
      )
    ).toBe(404)
    await page.goto("/approvals")
    await expect(
      page.getByRole("heading", { name: "Page not found", exact: true })
    ).toBeVisible()
    await page.setViewportSize({ width: 1440, height: 1000 })
    await page.goto(`/sources/${sourceId}/accounts`)
    await page
      .getByLabel("Bearer token", { exact: true })
      .fill("unsaved-source-bound-secret")
    const others = await page.evaluate(
      (id) =>
        fetch("/api/sources")
          .then((r) => r.json())
          .then((sources) =>
            sources.filter(
              (source: Source) => source.id !== id && source.kind === "mcp_http"
            )
          ),
      sourceId
    )
    await page.evaluate((path) => {
      history.pushState({}, "", path)
      dispatchEvent(new PopStateEvent("popstate"))
    }, `/sources/${others[0].id}/accounts`)
    await expect(page.getByLabel(/bearer token/i)).toHaveValue("")
    await page.goto(`/sources/${sourceId}`)
    await page.getByLabel("Name", { exact: true }).fill("unsaved-name")
    await page.evaluate((path) => {
      history.pushState({}, "", path)
      dispatchEvent(new PopStateEvent("popstate"))
    }, `/sources/${others[0].id}`)
    await expect(page.getByLabel("Name", { exact: true })).toHaveValue(
      others[0].slug
    )
    await page.setViewportSize({ width: 390, height: 844 })
    await page.goto(`/sources/${sourceId}/accounts`)
    await page.getByLabel("Credential type").selectOption("basic")
    await noOverflow(page)
    await capture(page, "openapi-account-mobile.png")
  } finally {
    await page.evaluate(async (id) => {
      await fetch(`/api/tokens/${id}`, { method: "DELETE" })
    }, issued.id)
  }
})

test("GraphQL: add a source, link a credential, scope its tools and execute an allowed mutation", async ({
  page,
}) => {
  await login(page, "/sources/new")
  await page.getByLabel("Name", { exact: true }).fill("browser-graphql")
  await page.getByLabel("Connection type").selectOption("graphql")
  await page
    .getByLabel("GraphQL endpoint address")
    .fill(fixtureUrl.replace(/\/mcp$/, "/graphql"))
  await page
    .getByRole("checkbox", { name: /Require a linked account/i })
    .check()
  await capture(page, "graphql-new-source.png")
  await page.getByRole("button", { name: "Add source", exact: true }).click()
  await expect(page).toHaveURL(/\/sources\/[0-9a-f-]+$/)
  const sourceId = new URL(page.url()).pathname.split("/").at(-1)!
  await page.getByRole("link", { name: "Manage account", exact: true }).click()
  await expect(
    page.getByRole("button", { name: "Connect with OAuth" })
  ).toHaveCount(0)
  await page.getByLabel("Credential type").selectOption("bearer")
  await page
    .getByLabel("Bearer token", { exact: true })
    .fill("browser-graphql-secret")
  await page
    .getByRole("button", { name: "Save credential", exact: true })
    .click()
  await expect(page.getByText("HTTP credential linked")).toBeVisible()
  await expect(page.getByLabel("Bearer token", { exact: true })).toHaveValue("")
  await page.reload()
  await expect(page.getByText("HTTP credential linked")).toBeVisible()
  await capture(page, "graphql-account-desktop.png")
  await page.setViewportSize({ width: 390, height: 844 })
  await noOverflow(page)
  await capture(page, "graphql-account-mobile.png")
  await page.setViewportSize({ width: 1440, height: 1000 })
  const source = await page.evaluate(
    (id) => fetch(`/api/sources/${id}`).then((r) => r.json()),
    sourceId
  )
  const main = await page.evaluate(() =>
    fetch("/api/workspaces")
      .then((r) => r.json())
      .then((es) => es.find((e: { allSources: boolean }) => e.allSources))
  )
  await page.goto(`/workspaces/${main.id}/policies`)
  const include = async (name: string) => {
    const response = page.waitForResponse(
      (r) =>
        new URL(r.url()).pathname ===
          `/api/workspaces/${main.id}/scope/tools` &&
        r.request().method() === "PUT"
    )
    await page
      .getByRole("checkbox", {
        name: `Include ${source.pythonName}.${name}`,
        exact: true,
      })
      .check()
    expect((await response).status()).toBe(204)
    await expect(
      page.getByRole("checkbox", {
        name: `Include ${source.pythonName}.${name}`,
        exact: true,
      })
    ).toBeEnabled()
  }
  await include("query")
  await include("mutation")
  const issued = await page.evaluate(async () =>
    fetch("/api/tokens", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        name: "GraphQL browser check",
        workspaceId: (
          await fetch("/api/workspaces").then((r) => r.json())
        ).find((w: { allSources: boolean }) => w.allSources).id,
      }),
    }).then((r) => r.json())
  )
  const invoke = async (name: string, args: unknown) => {
    const response = await page.request.post(`${loopback}/mcp`, {
      headers: {
        host: hostname,
        Authorization: `Bearer ${issued.token}`,
        Accept: "application/json, text/event-stream",
        "MCP-Protocol-Version": "2025-11-25",
      },
      data: {
        jsonrpc: "2.0",
        id: 1,
        method: "tools/call",
        params: { name, arguments: args },
      },
    })
    expect(response.ok()).toBe(true)
    return (await response.json()).result
  }
  try {
    const read = JSON.parse(
      (
        await invoke("execute", {
          code: `return await tools.${source.pythonName}.query(query='query Read($id:ID!) { item(id:$id) { id } }',variables={'id':'one'})`,
        })
      ).content[0].text
    )
    expect(read.status).toBe("completed")
    expect(read.result).toEqual({ kind: "query", variables: { id: "one" } })
    const before = graphqlMutations
    const mutation = JSON.parse(
      (
        await invoke("execute", {
          code: `return await tools.${source.pythonName}.mutation(query='mutation { deleteItem }')`,
        })
      ).content[0].text
    )
    expect(mutation.status).toBe("completed")
    expect(mutation.result.kind).toBe("mutation")
    expect(graphqlMutations).toBe(before + 1)
  } finally {
    await page.evaluate(
      (id) => fetch(`/api/tokens/${id}`, { method: "DELETE" }),
      issued.id
    )
  }
})

test("Saved scripts: CRUD, stale-edit protection and both MCP eras", async ({
  page,
}) => {
  await login(page, "/workspaces")
  const main = await page.evaluate(() =>
    fetch("/api/workspaces")
      .then((r) => r.json())
      .then((es) => es.find((e: { allSources: boolean }) => e.allSources))
  )
  await page.goto(`/workspaces/${main.id}`)
  await page.getByRole("link", { name: "Saved scripts", exact: true }).click()
  await expect(
    page.getByText("No saved scripts", { exact: true })
  ).toBeVisible()
  await page.getByRole("link", { name: "Create script", exact: true }).click()
  await page.getByLabel("Name", { exact: true }).fill("Weekly issues")
  await page
    .getByLabel("Description", { exact: true })
    .fill("Summarize project issues for weekly review")
  await page
    .getByLabel("Python source", { exact: true })
    .fill("print('saved run')\nreturn 42")
  await page.getByRole("button", { name: "Create script", exact: true }).click()
  await expect(page).toHaveURL(/\/scripts\/[0-9a-f-]+$/)
  const scriptId = new URL(page.url()).pathname.split("/").at(-1)!
  await page.reload()
  await expect(page.getByLabel("Python source", { exact: true })).toHaveValue(
    "print('saved run')\nreturn 42"
  )
  await noOverflow(page)
  await capture(page, "scripts-editor-desktop.png")
  await page
    .getByRole("link", { name: "All saved scripts", exact: true })
    .click()
  await expect(
    page.getByRole("link", { name: "Weekly issues", exact: true })
  ).toBeVisible()
  await capture(page, "scripts-list-desktop.png")
  await page.setViewportSize({ width: 390, height: 844 })
  await noOverflow(page)
  await capture(page, "scripts-list-mobile.png")
  await page.getByRole("link", { name: "Weekly issues", exact: true }).click()
  await noOverflow(page)
  await capture(page, "scripts-editor-mobile.png")
  await page
    .getByLabel("Python source", { exact: true })
    .fill("return 'my unsaved draft'")
  const path = `/api/workspaces/${main.id}/scripts/${scriptId}`
  const concurrent = await page.evaluate(async (path) => {
    const original = await fetch(path).then((r) => r.json())
    const r = await fetch(path, {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        ...original,
        id: undefined,
        updatedAt: undefined,
        code: "return 43",
      }),
    })
    return { status: r.status, body: await r.json() }
  }, path)
  expect(concurrent.status).toBe(200)
  await page.getByRole("button", { name: "Save changes", exact: true }).click()
  await expect(page.getByRole("alert")).toContainText("script changed")
  await expect(page.getByLabel("Python source", { exact: true })).toHaveValue(
    "return 'my unsaved draft'"
  )
  await capture(page, "scripts-conflict-mobile.png")
  await page.reload()
  await expect(page.getByLabel("Python source", { exact: true })).toHaveValue(
    "return 43"
  )
  await page.getByLabel("Python source", { exact: true }).fill("return 44")
  await page.getByRole("button", { name: "Save changes", exact: true }).click()
  await expect(page.getByRole("status")).toHaveText("Changes saved.")
  const issued = await page.evaluate(async () =>
    fetch("/api/tokens", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        name: "Saved script browser check",
        workspaceId: (
          await fetch("/api/workspaces").then((r) => r.json())
        ).find((w: { allSources: boolean }) => w.allSources).id,
      }),
    }).then((r) => r.json())
  )
  const rpc = async (
    version: string,
    method: string,
    params: Record<string, unknown>
  ) => {
    const headers: Record<string, string> = {
      host: hostname,
      Authorization: `Bearer ${issued.token}`,
      Accept: "application/json, text/event-stream",
      "MCP-Protocol-Version": version,
    }
    if (version === "2026-07-28") {
      headers["Mcp-Method"] = method
      if (method === "tools/call") headers["Mcp-Name"] = String(params.name)
      params = {
        ...params,
        _meta: {
          "io.modelcontextprotocol/protocolVersion": version,
          "io.modelcontextprotocol/clientCapabilities": {},
          "io.modelcontextprotocol/clientInfo": {
            name: "scripts-browser",
            version: "1",
          },
        },
      }
    }
    const response = await page.request.post(`${loopback}/mcp`, {
      headers,
      data: { jsonrpc: "2.0", id: 1, method, params },
    })
    expect(response.ok(), await response.text()).toBe(true)
    const body = await response.json()
    expect(body.error).toBeUndefined()
    return body.result
  }
  try {
    for (const version of ["2025-11-25", "2026-07-28"]) {
      const tools = await rpc(version, "tools/list", {})
      expect(
        tools.tools.map((tool: { name: string }) => tool.name).sort()
      ).toEqual(["execute", "search"])
      const found = await rpc(version, "tools/call", {
        name: "search",
        arguments: { query: "issues" },
      })
      const matches = JSON.parse(found.content[0].text).scripts
      expect(matches).toEqual([
        {
          id: scriptId,
          name: "Weekly issues",
          description: "Summarize project issues for weekly review",
        },
      ])
      const result = await rpc(version, "tools/call", {
        name: "execute",
        arguments: { scriptId },
      })
      expect(result.isError).not.toBe(true)
      expect(JSON.parse(result.content[0].text).result).toBe(44)
      for (const args of [
        {},
        { code: "return 1", scriptId },
        { scriptId: "bad-uuid" },
      ]) {
        expect(
          (
            await rpc(version, "tools/call", {
              name: "execute",
              arguments: args,
            })
          ).isError
        ).toBe(true)
      }
    }
  } finally {
    await page.evaluate(
      (id) => fetch(`/api/tokens/${id}`, { method: "DELETE" }),
      issued.id
    )
  }
  await page.getByRole("button", { name: "Delete script", exact: true }).click()
  await expect(page.getByRole("alertdialog")).toHaveCount(0)
  await expect(page.getByRole("status")).toContainText("within 3 seconds")
  await page.getByRole("button", { name: "Delete script", exact: true }).click()
  await expect(page).toHaveURL(`/workspaces/${main.id}/scripts`)
  await expect(
    page.getByText("No saved scripts", { exact: true })
  ).toBeVisible()
})

test("each account keeps its own MCPs, credentials, agent access and history", async ({
  page,
}) => {
  await login(page)
  const owner = await page.evaluate(async () => {
    const sources = await fetch("/api/sources").then((r) => r.json())
    const workspaces = await fetch("/api/workspaces").then((r) => r.json())
    const executions = await fetch("/api/executions").then((r) => r.json())
    const source = sources.find((s: Source) => s.slug === "browser-fixture")
    const main = workspaces.find((e: { allSources: boolean }) => e.allSources)
    const scope = await fetch(`/api/workspaces/${main.id}/scope`, {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ allTools: true }),
    })
    if (scope.status !== 204)
      throw new Error("Could not prepare owner endpoint scope")
    const token = await fetch("/api/tokens", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        name: "Tenant owner check",
        workspaceId: main.id,
      }),
    }).then((r) => r.json())
    return { source, main, execution: executions[0], token }
  })
  const rpc = async (token: string, name: string, args: object) => {
    const response = await page.request.post(`${loopback}/mcp`, {
      headers: {
        host: hostname,
        Authorization: `Bearer ${token}`,
        Accept: "application/json, text/event-stream",
        "MCP-Protocol-Version": "2025-11-25",
      },
      data: {
        jsonrpc: "2.0",
        id: 1,
        method: "tools/call",
        params: { name, arguments: args },
      },
    })
    expect(response.ok(), await response.text()).toBe(true)
    const body = await response.json()
    return {
      error: body.result.isError,
      value: JSON.parse(body.result.content[0].text),
    }
  }
  await page.getByRole("button", { name: /^Account menu for/ }).click()
  await page.getByRole("menuitem", { name: "Sign out", exact: true }).click()
  await page
    .getByRole("button", { name: "Create an account", exact: true })
    .click()
  await page
    .getByLabel("Email", { exact: true })
    .fill("second@olegpustovit.com")
  await page.getByLabel("Password", { exact: true }).fill(password)
  await page
    .getByRole("button", { name: "Create account", exact: true })
    .click()
  await expect(page).toHaveURL(`${origin}/integrations`)
  await expect(
    page.getByRole("heading", { name: "Integrations", exact: true })
  ).toBeVisible()
  await expect(
    page.getByText("second@olegpustovit.com", { exact: true })
  ).toBeVisible()
  const denied = await page.evaluate(
    async ({ source, main, execution, token }) => {
      const get = (p: string) => fetch(p).then((r) => r.status)
      const write = (p: string, body: object) =>
        fetch(p, {
          method: "PUT",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(body),
        }).then((r) => r.status)
      return {
        source: await get(`/api/sources/${source.id}`),
        endpoint: await get(`/api/workspaces/${main.id}`),
        execution: await get(`/api/executions/${execution.id}`),
        session: await get(`/api/sessions/${execution.sessionId}`),
        renameSession: await fetch(`/api/sessions/${execution.sessionId}`, {
          method: "PATCH",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ title: "foreign rename" }),
        }).then((r) => r.status),
        sessions: await fetch("/api/sessions").then((r) => r.json()),
        credential: await write(`/api/accounts/${source.id}`, {
          credential: "foreign-replacement",
        }),
        token: await fetch(`/api/tokens/${token.id}`, {
          method: "DELETE",
        }).then((r) => r.status),
        user: await fetch("/api/session")
          .then((r) => r.json())
          .then((s) => s.user),
        sources: await fetch("/api/sources").then((r) => r.json()),
        accounts: await fetch("/api/accounts").then((r) => r.json()),
        history: await fetch("/api/executions").then((r) => r.json()),
      }
    },
    owner
  )
  expect(denied.source).toBe(404)
  expect(denied.endpoint).toBe(404)
  expect(denied.execution).toBe(404)
  expect(denied.session).toBe(404)
  expect(denied.renameSession).toBe(404)
  expect(denied.sessions.sessions).toEqual([])
  expect(denied.credential).toBe(404)
  expect(denied.token).toBe(404)
  expect(denied.user.admin).toBe(false)
  expect(denied.sources).toEqual([])
  expect(denied.accounts).toEqual([])
  expect(denied.history).toEqual([])
  const secondary = await page.evaluate(async (url) => {
    const write = async (path: string, method: string, body: object) => {
      const r = await fetch(path, {
        method,
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body),
      })
      if (!r.ok) throw new Error(`Request failed ${r.status}`)
      return r.status === 204 ? null : r.json()
    }
    const source = await write("/api/sources", "POST", {
      slug: "browser-fixture",
      kind: "mcp_http",
      url,
    })
    await write(`/api/accounts/${source.id}`, "PUT", {
      credential: "tenant-b-secret",
    })
    const workspaces = await fetch("/api/workspaces").then((r) => r.json())
    const main = workspaces.find((e: { allSources: boolean }) => e.allSources)
    const token = await write("/api/tokens", "POST", {
      name: "Second agent",
      workspaceId: main.id,
    })
    return { source, main, token }
  }, fixtureUrl)
  expect(secondary.main.id).not.toBe(owner.main.id)
  await expect
    .poll(() =>
      page.evaluate(
        (id) =>
          fetch(`/api/sources/${id}`)
            .then((r) => r.json())
            .then((s) => s.status),
        secondary.source.id
      )
    )
    .toBe("ready")
  const search = await rpc(secondary.token.token, "search", { query: "" })
  expect(search.value.tools.map((t: { path: string }) => t.path)).toEqual([
    "browser_fixture.echo",
  ])
  const secondCall = await rpc(secondary.token.token, "execute", {
    code: 'return await tools.browser_fixture.echo(value="check")',
  })
  expect(secondCall.value.status).toBe("completed")
  expect(JSON.stringify(secondCall.value.result)).toContain("second-tenant")
  const firstCall = await rpc(owner.token.token, "execute", {
    code: 'return await tools.browser_fixture.echo(value="check")',
  })
  expect(firstCall.value.status, firstCall.value.error).toBe("completed")
  expect(JSON.stringify(firstCall.value.result)).toContain("first-tenant")
  expect(
    await page.evaluate(
      (id) => fetch(`/api/executions/${id}`).then((r) => r.status),
      firstCall.value.executionId
    )
  ).toBe(404)
  await page.reload()
  await expect(
    page.getByRole("row", { name: "browser-fixture", exact: true })
  ).toBeVisible()
  await capture(page, "tenant-secondary.png")
  await restartGateway()
  expect(
    (await rpc(secondary.token.token, "search", { query: "" })).value.tools
  ).toHaveLength(1)
  await page.getByRole("button", { name: /^Account menu for/ }).click()
  await page.getByRole("menuitem", { name: "Sign out", exact: true }).click()
  await login(page)
  await expect(
    page.getByText("ursus95@gmail.com", { exact: true })
  ).toBeVisible()
  expect(
    await page.evaluate(
      (id) => fetch(`/api/sources/${id}`).then((r) => r.status),
      secondary.source.id
    )
  ).toBe(404)
})

test("workspace bookmarks, preset connection and separate administration authorization", async ({
  page,
}) => {
  await login(page, "/workspaces")
  await page.goto("/endpoints")
  await expect(page).toHaveURL(`${origin}/workspaces`)
  await page.goto("/sources")
  await expect(
    page.getByRole("heading", { name: "Integrations", exact: true })
  ).toBeVisible()
  await capture(page, "workspace-source-presets-desktop.png")
  await page.setViewportSize({ width: 390, height: 844 })
  await noOverflow(page)
  await capture(page, "workspace-source-presets-mobile.png")
  const sourceId = await page.evaluate(async () => {
    const response = await fetch("/api/integrations/notion", { method: "POST" })
    if (!response.ok) throw new Error("Could not add registry integration")
    return (await response.json()).id as string
  })
  await page.goto(`/integrations?selected=notion&detail=open`)
  const preset = await page.evaluate(
    (id) => fetch(`/api/sources/${id}`).then((r) => r.json()),
    sourceId
  )
  expect(preset.url).toBe("https://mcp.notion.com/mcp")
  expect(preset.requiresAccount).toBe(true)
  await expect(
    page.getByRole("button", { name: "Connect", exact: true })
  ).toBeVisible()
  // No provider sign-in or external consent is performed by this test.
  await page.evaluate(
    (id) => fetch(`/api/sources/${id}`, { method: "DELETE" }),
    sourceId
  )
  await page.setViewportSize({ width: 1440, height: 1000 })
  await page.goto("/tokens")
  await page
    .locator(".list-heading")
    .getByRole("button", { name: "New personal token", exact: true })
    .click()
  await page.getByLabel("Access", { exact: true }).selectOption("admin")
  await expect(page.getByLabel("Workspace", { exact: true })).toHaveCount(0)
  await page
    .getByLabel("Name", { exact: true })
    .fill("Management browser check")
  await page.getByRole("button", { name: "Create token", exact: true }).click()
  const token = await page.getByLabel("New access token").inputValue()
  await expect(
    page
      .locator(".code-panel")
      .filter({ hasText: "codex mcp add codemode-admin" })
  ).toContainText(
    "https://codemode.microapps.space/mcp-admin --bearer-token-env-var CODEMODE_TOKEN"
  )
  await expect(
    page.getByText("code_mode_tool_search = false", { exact: false })
  ).toHaveCount(0)
  const rpc = async (resource: string, method: string, params: object) => {
    const response = await page.request.post(`${loopback}${resource}`, {
      headers: {
        host: hostname,
        Authorization: `Bearer ${token}`,
        Accept: "application/json, text/event-stream",
        "MCP-Protocol-Version": "2025-11-25",
      },
      data: { jsonrpc: "2.0", id: 1, method, params },
    })
    return response
  }
  expect((await rpc("/mcp", "tools/list", {})).status()).toBe(401)
  const tools = await (await rpc("/mcp-admin", "tools/list", {})).json()
  expect(tools.result.tools).toHaveLength(10)
  const modern = await page.request.post(`${loopback}/mcp-admin`, {
    headers: {
      host: hostname,
      Authorization: `Bearer ${token}`,
      Accept: "application/json, text/event-stream",
      "MCP-Protocol-Version": "2026-07-28",
      "Mcp-Method": "tools/list",
    },
    data: {
      jsonrpc: "2.0",
      id: 1,
      method: "tools/list",
      params: {
        _meta: {
          "io.modelcontextprotocol/protocolVersion": "2026-07-28",
          "io.modelcontextprotocol/clientCapabilities": {},
          "io.modelcontextprotocol/clientInfo": {
            name: "management-browser",
            version: "1",
          },
        },
      },
    },
  })
  expect(modern.status()).toBe(200)
  expect((await modern.json()).result.tools).toHaveLength(10)
  const workspaces = await (
    await rpc("/mcp-admin", "tools/call", {
      name: "list_workspaces",
      arguments: {},
    })
  ).json()
  expect(
    JSON.parse(workspaces.result.content[0].text).some(
      (w: { allSources: boolean }) => w.allSources
    )
  ).toBe(true)
  await page.getByRole("button", { name: "I saved it", exact: true }).click()
  const managementRow = page
    .getByRole("grid", { name: "Agents with access", exact: true })
    .getByRole("row", { name: "Management browser check", exact: true })
  await expect(managementRow).toContainText("Administration")
  await managementRow.click()
  const managementDetails = page.getByRole("region", {
    name: "Token details",
    exact: true,
  })
  await expect(managementDetails).toContainText("Administration")
  await expect(managementDetails).toContainText("/mcp-admin")
  const registration = await page.evaluate(() =>
    fetch("/oauth/register", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        client_name: "Workspace manager",
        redirect_uris: ["https://client.example/callback"],
        token_endpoint_auth_method: "none",
      }),
    }).then((r) => r.json())
  )
  const query = new URLSearchParams({
    response_type: "code",
    client_id: registration.client_id,
    redirect_uri: "https://client.example/callback",
    resource: `${origin}/mcp-admin`,
    scope: "admin",
    code_challenge: createHash("sha256")
      .update("browser-verifier-123456789012345678901234567890123456789")
      .digest("base64url"),
    code_challenge_method: "S256",
  })
  await page.goto(`/oauth/authorize?${query}`)
  await expect(
    page.getByRole("heading", { name: "Authorize Workspace manager?" })
  ).toBeVisible()
  await expect(page.getByLabel("Workspace", { exact: true })).toHaveCount(0)
  await expect(page.getByRole("alert")).toContainText(
    "create, change and delete your workspaces"
  )
  await capture(page, "workspace-admin-consent-desktop.png")
  await page.setViewportSize({ width: 390, height: 844 })
  await noOverflow(page)
  await capture(page, "workspace-admin-consent-mobile.png")
  await page
    .getByRole("button", { name: "Authorize administration", exact: true })
    .click()
  await expect(page).toHaveURL(/client\.example\/callback\?.*code=/)
})

test("integrations: real catalogue search, URL-backed panes and keyboard focus", async ({
  page,
}) => {
  const errors: string[] = []
  page.on("pageerror", (error) => errors.push(error.message))
  await page.goto("/login?redirect=%2Fintegrations")
  await expect(
    page.getByRole("heading", {
      name: /^(Create your account|Sign in to your control plane)$/,
    })
  ).toBeVisible()
  const first = await page
    .getByRole("heading", { name: "Create your account", exact: true })
    .isVisible()
  await page.getByLabel("Email", { exact: true }).fill("ursus95@gmail.com")
  await page.getByLabel("Password", { exact: true }).fill(password)
  await page
    .getByRole("button", {
      name: first ? "Create account" : "Sign in",
      exact: true,
    })
    .click()
  await expect(page).toHaveURL(`${origin}/integrations`)
  await measurePaneLatency(
    page,
    resolve(
      root,
      process.env.CODEMODE_LATENCY_BASELINE
        ? "docs/measurements/pane-baseline.json"
        : "target/ui-browser/pane-latency.json"
    )
  )
  const sourceId = await page.evaluate(async (url) => {
    const response = await fetch("/api/sources", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        slug: "keyboard-integration",
        kind: "mcp_http",
        url,
        requiresAccount: false,
      }),
    })
    if (!response.ok) throw new Error("Could not create keyboard fixture")
    return (await response.json()).id as string
  }, fixtureUrl)
  try {
    await expect
      .poll(() =>
        page.evaluate(async (id) => {
          const entries = await fetch("/api/integrations").then((r) => r.json())
          return (
            entries.find((entry: { sourceId: string }) => entry.sourceId === id)
              ?.toolNames ?? []
          )
        }, sourceId)
      )
      .toContain("keyboard_integration.echo")
    await page.setViewportSize({ width: 390, height: 600 })
    const scrolling = await page
      .getByRole("grid", { name: "Integrations", exact: true })
      .evaluate((grid) => {
        const body = grid.querySelector<HTMLElement>(".data-body")!
        const header = grid.querySelector("thead")!
        const top = header.getBoundingClientRect().top
        body.scrollTop = body.scrollHeight
        return {
          scrolled: body.scrollTop > 0,
          headerFixed: header.getBoundingClientRect().top === top,
        }
      })
    expect(scrolling).toEqual({ scrolled: true, headerFixed: true })
    await page.setViewportSize({ width: 1440, height: 1000 })
    await page.goto(`/integrations?selected=${sourceId}&detail=open`)
    await expect(
      page.getByRole("heading", { name: "keyboard-integration", exact: true })
    ).toBeVisible()
    const search = page.getByRole("searchbox", { name: "Search this list" })
    await page.keyboard.press("Backslash")
    await expect(
      page.getByRole("button", { name: "Expand sidebar" })
    ).toBeVisible()
    await page.reload()
    const railToggle = page.getByRole("button", { name: "Expand sidebar" })
    await expect(railToggle).toBeVisible()
    await search.focus()
    await page.keyboard.press("Backslash")
    await expect(railToggle).toBeVisible()
    await railToggle.click()
    await expect(
      page.getByRole("button", { name: "Collapse sidebar" })
    ).toBeVisible()
    await search.fill("keyboard integration echo")
    const grid = page.getByRole("grid", { name: "Integrations", exact: true })
    await expect(
      grid.getByRole("row", { name: "keyboard-integration", exact: true })
    ).toBeVisible()
    await expect(
      grid.getByRole("row", { name: "Notion", exact: true })
    ).toHaveCount(0)
    await search.fill("")
    await grid.focus()
    await page.keyboard.press("ArrowLeft")
    await expect(
      page.getByRole("heading", { name: "keyboard-integration", exact: true })
    ).toBeHidden()
    await page.keyboard.press("Enter")
    await expect(
      page.getByRole("heading", { name: "keyboard-integration", exact: true })
    ).toBeVisible()
    await page.reload()
    await expect(
      page.getByRole("heading", { name: "keyboard-integration", exact: true })
    ).toBeVisible()
    await capture(page, "integrations-panes-desktop.png")
    await page.setViewportSize({ width: 390, height: 844 })
    await noOverflow(page)
    await capture(page, "integrations-panes-mobile.png")
    await page
      .getByRole("navigation", { name: "Sections" })
      .getByRole("link", { name: "Workspaces", exact: true })
      .focus()
    await page.keyboard.press("Escape")
    await expect(
      page.getByRole("navigation", { name: "Sections" })
    ).toBeVisible()
    await expect(
      page.getByRole("heading", { name: "keyboard-integration", exact: true })
    ).toBeVisible()
    await page.getByRole("button", { name: /^Close/ }).click()
    await expect(grid).toBeFocused()
    await noOverflow(page)
    await page.setViewportSize({ width: 1440, height: 1000 })
    await page.goto("/workspaces?q=keyboard_integration")
    const tools = page.getByRole("grid", {
      name: "Workspace tools",
      exact: true,
    })
    await expect(
      tools.getByRole("row", { name: "keyboard_integration.echo", exact: true })
    ).toBeVisible()
    await tools.focus()
    await page.keyboard.press("Enter")
    const policy = page.getByRole("region", {
      name: "Workspace tool details",
      exact: true,
    })
    await expect(
      policy.getByRole("heading", {
        name: "keyboard_integration.echo",
        exact: true,
      })
    ).toBeVisible()
    await expect(policy.locator("pre")).toContainText("async def")
    await policy.getByRole("button", { name: /^Block all from source/ }).click()
    await expect(
      tools.getByRole("row", { name: "keyboard_integration.echo", exact: true })
    ).toContainText("Blocked")
    await policy.getByRole("button", { name: /^Allow all from source/ }).click()
    await expect(
      tools.getByRole("row", { name: "keyboard_integration.echo", exact: true })
    ).toContainText("Allowed")
    await policy.getByRole("button", { name: /^Block Space/ }).focus()
    await page.keyboard.press("Space")
    await expect(
      tools.getByRole("row", { name: "keyboard_integration.echo", exact: true })
    ).toContainText("Blocked")
    await page.reload()
    await expect(
      policy.getByRole("heading", {
        name: "keyboard_integration.echo",
        exact: true,
      })
    ).toBeVisible()
    const blockedSwitch = tools.getByRole("switch", {
      name: "Blocked: keyboard_integration.echo",
      exact: true,
    })
    await expect(blockedSwitch).toHaveAttribute("aria-checked", "false")
    expect(
      await blockedSwitch
        .locator("i")
        .evaluate((node) => getComputedStyle(node).width)
    ).toBe("26px")
    await capture(page, "workspaces-panes-desktop.png")
    await page.setViewportSize({ width: 390, height: 844 })
    await noOverflow(page)
    await capture(page, "workspaces-panes-mobile.png")
    await policy.getByRole("button", { name: /Close/ }).click()
    await expect(tools).toBeFocused()
    expect(errors).toEqual([])
  } finally {
    await page.evaluate(
      (id) => fetch(`/api/sources/${id}`, { method: "DELETE" }),
      sourceId
    )
  }
})

test("OAuth apps: write-only setup, static PKCE redirect and guarded removal", async ({
  page,
}) => {
  const errors: string[] = []
  page.on("pageerror", (error) => errors.push(error.message))
  await page.goto("/login?redirect=%2Fintegrations")
  await expect(
    page.getByRole("heading", {
      name: /^(Create your account|Sign in to your control plane)$/,
    })
  ).toBeVisible()
  const first = await page
    .getByRole("heading", { name: "Create your account", exact: true })
    .isVisible()
  await page.getByLabel("Email", { exact: true }).fill("ursus95@gmail.com")
  await page.getByLabel("Password", { exact: true }).fill(password)
  await page
    .getByRole("button", {
      name: first ? "Create account" : "Sign in",
      exact: true,
    })
    .click()
  await expect(page).toHaveURL(`${origin}/integrations`)
  await page.goto("/integrations?selected=github&detail=open")
  await expect(
    page.getByRole("heading", { name: "GitHub", exact: true })
  ).toBeVisible()
  await page
    .getByLabel("Connection method", { exact: true })
    .selectOption("oauth_app")
  await expect(
    page.getByText(`${origin}/api/accounts/oauth/callback`, { exact: true })
  ).toBeVisible()
  await expect(
    page.getByRole("button", { name: "Connect", exact: true })
  ).toHaveCount(0)
  await page.getByLabel("Client ID", { exact: true }).fill("browser-app-id")
  await page
    .getByLabel("Client secret", { exact: true })
    .fill("browser-app-secret")
  await page
    .getByRole("button", { name: "Save OAuth app", exact: true })
    .click()
  await expect(
    page.getByRole("button", { name: "Replace OAuth app", exact: true })
  ).toBeVisible()
  expect(
    await page.evaluate(() => fetch("/api/oauth-apps").then((r) => r.text()))
  ).toBe('["github"]')
  await page.reload()
  await page
    .getByLabel("Connection method", { exact: true })
    .selectOption("oauth_app")
  await page
    .getByRole("button", { name: "Replace OAuth app", exact: true })
    .click()
  await expect(page.getByLabel("Client ID", { exact: true })).toHaveValue("")
  await expect(page.getByLabel("Client secret", { exact: true })).toHaveValue(
    ""
  )
  await page.getByRole("button", { name: "Cancel", exact: true }).click()
  await capture(page, "oauth-app-desktop.png")
  await page.setViewportSize({ width: 390, height: 844 })
  await noOverflow(page)
  await capture(page, "oauth-app-mobile.png")
  // Intercept the provider page: this checks the redirect without contacting GitHub or consenting.
  await page.route("https://github.com/**", (route) =>
    route.fulfill({
      contentType: "text/html",
      body: "<p>Provider authorization fixture</p>",
    })
  )
  await page.getByRole("button", { name: "Connect", exact: true }).click()
  await expect(page).toHaveURL(
    /^https:\/\/github\.com\/login\/oauth\/authorize\?/
  )
  const authorization = new URL(page.url())
  expect(authorization.searchParams.get("client_id")).toBe("browser-app-id")
  expect(authorization.searchParams.get("code_challenge_method")).toBe("S256")
  expect(authorization.searchParams.get("code_challenge")).toMatch(
    /^[A-Za-z0-9_-]{43}$/
  )
  expect(authorization.searchParams.get("redirect_uri")).toBe(
    `${origin}/api/accounts/oauth/callback`
  )
  expect(authorization.searchParams.get("scope")).toContain("repo")
  expect(page.url()).not.toContain("browser-app-secret")
  const denied = new URL(`${origin}/api/accounts/oauth/callback`)
  denied.searchParams.set("state", authorization.searchParams.get("state")!)
  denied.searchParams.set("error", "access_denied")
  denied.searchParams.set(
    "error_description",
    "workspace policy rejected this app"
  )
  await page.goto(denied.href)
  await expect(page.getByRole("alert")).toContainText("step 5")
  await expect(page.getByRole("alert")).toContainText(
    "workspace policy rejected this app"
  )
  await page
    .getByLabel("Connection method", { exact: true })
    .selectOption("oauth_app")
  const remove = page.getByRole("button", {
    name: "Remove OAuth app",
    exact: true,
  })
  await remove.click()
  expect(
    await page.evaluate(() => fetch("/api/oauth-apps").then((r) => r.json()))
  ).toEqual(["github"])
  await remove.click()
  await expect(
    page.getByRole("button", { name: "Save OAuth app", exact: true })
  ).toBeVisible()
  expect(
    await page.evaluate(() => fetch("/api/oauth-apps").then((r) => r.json()))
  ).toEqual([])
  await expect(
    page.getByRole("button", { name: "Connect", exact: true })
  ).toHaveCount(0)
  const sourceId = await page.evaluate(async () => {
    const entries = await fetch("/api/integrations").then((r) => r.json())
    return entries.find((entry: { slug: string }) => entry.slug === "github")
      ?.sourceId as string | undefined
  })
  if (sourceId)
    await page.evaluate(
      (id) => fetch(`/api/sources/${id}`, { method: "DELETE" }),
      sourceId
    )
  expect(errors).toEqual([])
})

test("sessions: real execution grouping, rename, two panes and mobile focus", async ({
  page,
}) => {
  const errors: string[] = []
  page.on("pageerror", (error) => errors.push(error.message))
  await page.goto("/login?redirect=%2Fsessions")
  await expect(
    page.getByRole("heading", {
      name: /^(Create your account|Sign in to your control plane)$/,
    })
  ).toBeVisible()
  const first = await page
    .getByRole("heading", { name: "Create your account", exact: true })
    .isVisible()
  await page.getByLabel("Email", { exact: true }).fill("ursus95@gmail.com")
  await page.getByLabel("Password", { exact: true }).fill(password)
  await page
    .getByRole("button", {
      name: first ? "Create account" : "Sign in",
      exact: true,
    })
    .click()
  await expect(
    page.getByRole("heading", { name: "Sessions", exact: true })
  ).toBeVisible()
  const issued = await page.evaluate(async () =>
    fetch("/api/tokens", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        name: "Session browser agent",
        workspaceId: (
          await fetch("/api/workspaces").then((r) => r.json())
        ).find((w: { allSources: boolean }) => w.allSources).id,
      }),
    }).then((r) => r.json())
  )
  try {
    for (const code of [
      '# First investigation\nprint("captured stdout")\nreturn 42',
      '# Second investigation\nraise ValueError("Fixture failure")',
    ]) {
      const response = await page.request.post(`${loopback}/mcp`, {
        headers: {
          host: hostname,
          Authorization: `Bearer ${issued.token}`,
          Accept: "application/json, text/event-stream",
          "MCP-Protocol-Version": "2025-11-25",
        },
        data: {
          jsonrpc: "2.0",
          id: 1,
          method: "tools/call",
          params: { name: "execute", arguments: { code } },
        },
      })
      expect(response.ok(), await response.text()).toBe(true)
      expect((await response.json()).error).toBeUndefined()
    }
    await page.reload()
    const sessions = page.getByRole("grid", { name: "Sessions", exact: true })
    await expect(
      sessions.getByRole("row", { name: "First investigation", exact: true })
    ).toHaveCount(1)
    await sessions.focus()
    await page.keyboard.press("Enter")
    await page
      .getByRole("button", { name: "Rename session", exact: true })
      .click()
    await expect(page.getByLabel("Session title", { exact: true })).toHaveValue(
      "First investigation"
    )
    await page
      .getByLabel("Session title", { exact: true })
      .fill("Renamed browser session")
    await page.getByRole("button", { name: "Save title", exact: true }).click()
    await expect(
      page.getByRole("heading", {
        name: "Renamed browser session",
        exact: true,
      })
    ).toBeVisible()
    const runs = page.getByRole("grid", {
      name: "Session executions",
      exact: true,
    })
    await expect(runs.getByRole("row", { name: /investigation$/ })).toHaveCount(
      2
    )
    await runs.focus()
    await page.keyboard.press("Enter")
    const execution = page.getByRole("region", {
      name: "Execution details",
      exact: true,
    })
    await expect(
      execution.getByRole("heading", {
        name: "Second investigation",
        exact: true,
      })
    ).toBeVisible()
    await execution.getByRole("button", { name: /Close/ }).focus()
    const beforeArrow = page.url()
    await page.keyboard.press("j")
    await expect(
      execution.getByRole("heading", {
        name: "First investigation",
        exact: true,
      })
    ).toBeVisible()
    await expect(
      execution.getByText("captured stdout", { exact: true })
    ).toBeVisible()
    await expect(sessions).toHaveCount(0)
    await expect.poll(() => page.url()).not.toBe(beforeArrow)
    await page.reload()
    await expect(
      execution.getByRole("heading", {
        name: "First investigation",
        exact: true,
      })
    ).toBeVisible()
    await capture(page, "sessions-panes-desktop.png")
    const sessionDeepLink = page.url()
    await page
      .getByRole("link", { name: "Save as script", exact: true })
      .click()
    await expect(page.getByLabel("Python source", { exact: true })).toHaveValue(
      '# First investigation\nprint("captured stdout")\nreturn 42'
    )
    await expect(page.getByLabel("Name", { exact: true })).toHaveValue("")
    await page.getByLabel("Name", { exact: true }).fill("from-session-browser")
    await page
      .getByRole("button", { name: "Create script", exact: true })
      .click()
    await expect(
      page.getByRole("heading", { name: "Edit script", exact: true })
    ).toBeVisible()
    await page
      .getByRole("heading", { name: "Edit script", exact: true })
      .click()
    await page.keyboard.press("4")
    await expect(
      page.getByRole("heading", { name: "Scripts", exact: true })
    ).toBeVisible()
    const scriptGrid = page.getByRole("grid", { name: "Scripts", exact: true })
    await expect(
      scriptGrid.getByRole("row", { name: "from-session-browser", exact: true })
    ).toBeVisible()
    const scope = await page.evaluate(() =>
      fetch("/api/workspaces", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ name: "Script scope fixture", sourceIds: [] }),
      }).then((r) => r.json())
    )
    await page.reload()
    await page
      .getByRole("button", { name: "Script scope fixture", exact: true })
      .click()
    await expect(
      scriptGrid.getByRole("row", { name: "from-session-browser", exact: true })
    ).toHaveCount(0)
    await page.getByRole("heading", { name: "Scripts", exact: true }).click()
    await page.keyboard.press("3")
    await expect(
      page.getByRole("tab", { name: /^Script scope fixture/ })
    ).toHaveAttribute("aria-selected", "true")
    await page
      .getByRole("heading", { name: "Script scope fixture", exact: true })
      .click()
    await page.keyboard.press("4")
    await expect(
      scriptGrid.getByRole("row", { name: "from-session-browser", exact: true })
    ).toHaveCount(0)
    await page.getByRole("button", { name: "All sources", exact: true }).click()
    await expect(
      scriptGrid.getByRole("row", { name: "from-session-browser", exact: true })
    ).toBeVisible()
    await page.evaluate(
      (id) => fetch(`/api/workspaces/${id}`, { method: "DELETE" }),
      scope.id
    )
    await page.reload()
    await expect(
      scriptGrid.getByRole("row", { name: "from-session-browser", exact: true })
    ).toBeVisible()
    await scriptGrid.focus()
    await page.keyboard.press("Enter")
    const scriptDetails = page.getByRole("region", {
      name: "Script details",
      exact: true,
    })
    await expect(scriptDetails.locator("pre")).toContainText("captured stdout")
    await page.reload()
    await expect(
      scriptDetails.getByRole("heading", {
        name: "from-session-browser",
        exact: true,
      })
    ).toBeVisible()
    await capture(page, "scripts-panes-desktop.png")
    await page.setViewportSize({ width: 390, height: 844 })
    await noOverflow(page)
    await capture(page, "scripts-panes-mobile.png")
    await scriptDetails
      .getByRole("button", { name: "Delete script", exact: true })
      .click()
    await expect(
      page.getByRole("button", { name: /Keep selection/ })
    ).toBeFocused()
    await scriptDetails
      .getByRole("button", { name: "Delete scripts", exact: true })
      .click()
    await expect(
      scriptGrid.getByRole("row", { name: "from-session-browser", exact: true })
    ).toHaveCount(0)
    await page.setViewportSize({ width: 1440, height: 1000 })
    await page.goto(sessionDeepLink)
    await expect(
      execution.getByRole("heading", {
        name: "First investigation",
        exact: true,
      })
    ).toBeVisible()
    await page.setViewportSize({ width: 390, height: 844 })
    await expect(execution).toBeVisible()
    await capture(page, "sessions-panes-mobile.png")
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth > innerWidth
      )
    ).toBe(false)
    await execution.getByRole("button", { name: /Close/ }).click()
    await expect(
      page.getByRole("heading", {
        name: "Renamed browser session",
        exact: true,
      })
    ).toBeVisible()
    await page
      .getByRole("region", { name: "Session details", exact: true })
      .getByRole("button", { name: /Close/ })
      .click()
    await expect(sessions).toBeFocused()
    await page.setViewportSize({ width: 1440, height: 1000 })
    await page.goto(`/tokens?selected=${issued.id}&detail=open`)
    const tokenDetails = page.getByRole("region", {
      name: "Token details",
      exact: true,
    })
    await expect(
      tokenDetails.getByRole("heading", {
        name: "Session browser agent",
        exact: true,
      })
    ).toBeVisible()
    await expect(tokenDetails).toContainText("Personal token")
    await page
      .locator(".list-heading")
      .getByRole("button", { name: "New personal token", exact: true })
      .click()
    await page.getByLabel("Name", { exact: true }).fill("Pane token fixture")
    await page
      .getByRole("button", { name: "Create token", exact: true })
      .click()
    await expect(page.getByLabel("New access token")).toHaveValue(/^cm_/)
    await page.getByRole("button", { name: "I saved it", exact: true }).click()
    await expect(page.getByLabel("New access token")).toHaveCount(0)
    const tokenGrid = page.getByRole("grid", {
      name: "Agents with access",
      exact: true,
    })
    await tokenGrid
      .getByRole("row", { name: "Pane token fixture", exact: true })
      .click()
    await expect(
      tokenDetails.getByRole("heading", {
        name: "Pane token fixture",
        exact: true,
      })
    ).toBeVisible()
    await capture(page, "tokens-panes-desktop.png")
    await page.setViewportSize({ width: 390, height: 844 })
    await noOverflow(page)
    await capture(page, "tokens-panes-mobile.png")
    await tokenDetails
      .getByRole("button", { name: "Revoke token", exact: true })
      .click()
    await tokenDetails
      .getByRole("button", { name: "Revoke token", exact: true })
      .click()
    await expect(tokenDetails).toContainText("Revoked")
    await page.reload()
    await expect(page.getByLabel("New access token")).toHaveCount(0)
    for (let index = 0; index < 21; index++) {
      const response = await page.request.post(`${loopback}/mcp`, {
        headers: {
          host: hostname,
          Authorization: `Bearer ${issued.token}`,
          Accept: "application/json, text/event-stream",
          "MCP-Protocol-Version": "2025-11-25",
        },
        data: {
          jsonrpc: "2.0",
          id: 1,
          method: "tools/call",
          params: {
            name: "execute",
            arguments: { code: `# Paging evidence ${index}\nreturn ${index}` },
          },
        },
      })
      expect(response.ok()).toBe(true)
    }
    await page.setViewportSize({ width: 1440, height: 1000 })
    await page.goto(sessionDeepLink)
    await expect(
      execution.getByRole("heading", {
        name: "First investigation",
        exact: true,
      })
    ).toBeVisible()
    await expect(
      page
        .getByRole("grid", { name: "Session executions", exact: true })
        .locator(".data-row")
    ).toHaveCount(23)
    await expect(
      page.getByRole("button", { name: "Older executions", exact: true })
    ).toHaveCount(0)
    expect(errors).toEqual([])
  } finally {
    await page.evaluate(
      (id) => fetch(`/api/tokens/${id}`, { method: "DELETE" }),
      issued.id
    )
  }
})

test("control-plane fixes: case, icons, stable scripts, desktop bulk editing, 300 tools and deliberate account actions", async ({
  page,
}) => {
  const errors: string[] = []
  page.on("pageerror", (error) => errors.push(error.message))
  await page.goto("/login?redirect=%2Fintegrations")
  await expect(
    page.getByRole("heading", {
      name: /^(Create your account|Sign in to your control plane)$/,
    })
  ).toBeVisible()
  const creating = await page
    .getByRole("heading", { name: "Create your account", exact: true })
    .isVisible()
  await page.getByLabel("Email", { exact: true }).fill("ursus95@gmail.com")
  await page.getByLabel("Password", { exact: true }).fill(password)
  await page
    .getByRole("button", {
      name: creating ? "Create account" : "Sign in",
      exact: true,
    })
    .click()
  await expect(page).toHaveURL(`${origin}/integrations`)
  const presets = await page.evaluate(async () =>
    fetch("/api/integrations").then((r) => r.json())
  )
  expect(
    presets
      .filter((entry: { slug: string }) => !entry.slug.startsWith("source-"))
      .map((entry: { slug: string }) => entry.slug)
      .sort()
  ).toEqual(["context7", "deepwiki", "github", "linear", "notion", "nudge"])
  const account = page.getByRole("button", { name: /^Account menu for/ })
  await account.click()
  await expect(
    page.getByRole("menuitem", { name: "Sign out", exact: true })
  ).toBeVisible()
  await capture(page, "fixes-account-menu-desktop.png")
  expect(
    (await page.evaluate(() => fetch("/api/session").then((r) => r.json())))
      .authenticated
  ).toBe(true)
  await page.keyboard.press("Escape")
  await expect(account).toBeFocused()
  await account.press("Enter")
  await page
    .getByRole("menuitem", { name: "Account settings", exact: true })
    .click()
  await expect(
    page.getByRole("heading", { name: "Account settings", exact: true })
  ).toBeVisible()
  await page.goto("/sources/new")
  await page.getByLabel("Name", { exact: true }).fill("Acme Billing")
  await page.getByRole("radio", { name: "card", exact: true }).check()
  await page
    .getByLabel("Server address", { exact: true })
    .fill(fixtureUrl.replace(/\/mcp$/, "/bulk-mcp"))
  await page.getByRole("button", { name: "Add source", exact: true }).click()
  await expect(
    page.getByRole("heading", { name: "Acme Billing", exact: true })
  ).toBeVisible()
  const sourceId = page.url().split("/").at(-1)!
  const workspace = await page.evaluate(async () => {
    const all = await fetch("/api/workspaces").then((r) => r.json())
    return all.find(
      (workspace: { allSources: boolean }) => workspace.allSources
    )
  })
  const owned = await page.evaluate(
    async ({ workspace }) => {
      const token = await fetch("/api/tokens", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          name: "Bulk fixture agent",
          workspaceId: workspace,
        }),
      }).then((r) => r.json())
      const script = await fetch(`/api/workspaces/${workspace}/scripts`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          name: "Stable namespace fixture",
          description: "Must survive source rename",
          code: "return await tools.acme_billing.get_item_150()",
        }),
      }).then((r) => r.json())
      const empty = await fetch("/api/workspaces", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ name: "Empty fixes fixture", sourceIds: [] }),
      }).then((r) => r.json())
      return { token, script, empty }
    },
    { workspace: workspace.id }
  )
  try {
    await expect
      .poll(() =>
        page.evaluate(
          (id) =>
            fetch(`/api/sources/${id}`)
              .then((r) => r.json())
              .then((source) => source.toolCount),
          sourceId
        )
      )
      .toBe(300)
    await page.goto("/integrations?q=Acme*")
    const integrations = page.getByRole("grid", {
      name: "Integrations",
      exact: true,
    })
    await expect(
      integrations
        .getByRole("row", { name: "Acme Billing", exact: true })
        .locator('[data-icon="card"]')
    ).toHaveCount(1)
    await integrations.focus()
    await page.keyboard.press("F2")
    const rename = page.getByRole("textbox", {
      name: "Rename Acme Billing",
      exact: true,
    })
    await rename.fill("ACME Finance")
    await rename.press("Enter")
    await expect(
      integrations.getByRole("row", { name: "ACME Finance", exact: true })
    ).toBeVisible()
    await page.reload()
    await expect(
      integrations.getByRole("row", { name: "ACME Finance", exact: true })
    ).toBeVisible()
    await integrations.focus()
    await page.keyboard.press("e")
    await expect(page.getByLabel("Name", { exact: true })).toHaveValue(
      "ACME Finance"
    )
    await expect(page.getByRole("radio")).toHaveCount(17)
    await page.getByRole("radio", { name: "database", exact: true }).check()
    await capture(page, "fixes-icon-picker-desktop.png")
    await page
      .getByRole("button", { name: "Save changes", exact: true })
      .click()
    await page.goto("/integrations?q=ACME*")
    await expect(
      integrations
        .getByRole("row", { name: "ACME Finance", exact: true })
        .locator('[data-icon="database"]')
    ).toHaveCount(1)
    await integrations
      .getByRole("row", { name: "ACME Finance", exact: true })
      .click()
    await expect(
      page
        .getByRole("region", { name: "Integration details", exact: true })
        .locator('[data-icon="database"]')
    ).toHaveCount(1)
    await capture(page, "fixes-connector-desktop.png")
    const source = await page.evaluate(
      (id) => fetch(`/api/sources/${id}`).then((r) => r.json()),
      sourceId
    )
    expect(source).toMatchObject({
      name: "ACME Finance",
      slug: "acme_billing",
      pythonName: "acme_billing",
      icon: "database",
    })
    const executed = await page.request.post(`${loopback}/mcp`, {
      headers: {
        host: hostname,
        Authorization: `Bearer ${owned.token.token}`,
        Accept: "application/json, text/event-stream",
        "MCP-Protocol-Version": "2025-11-25",
      },
      data: {
        jsonrpc: "2.0",
        id: 1,
        method: "tools/call",
        params: { name: "execute", arguments: { scriptId: owned.script.id } },
      },
    })
    const run = await executed.json()
    expect(run.error).toBeUndefined()
    expect(run.result.isError).not.toBe(true)
    expect(JSON.parse(run.result.content[0].text).result).toBe("fixture")
    await page.goto(`/workspaces?workspace=${workspace.id}&q=acme_billing.*`)
    const tools = page.getByRole("grid", {
      name: "Workspace tools",
      exact: true,
    })
    await expect(tools.locator(".data-row")).toHaveCount(50)
    await tools.focus()
    const before = page.url()
    for (let index = 0; index < 5; index++)
      await page.keyboard.press("Shift+ArrowDown")
    expect(page.url()).toBe(before)
    await expect(
      page.getByRole("heading", { name: "6 selected", exact: true })
    ).toBeVisible()
    await page.keyboard.press("b")
    await expect
      .poll(() =>
        page.evaluate(async (id) => {
          const page = await fetch(
            `/api/workspaces/${id}/policies?q=acme_billing.*`
          ).then((r) => r.json())
          return page.tools.filter(
            (tool: { effectiveAction: string }) =>
              tool.effectiveAction === "block"
          ).length
        }, workspace.id)
      )
      .toBe(6)
    await capture(page, "fixes-bulk-policies-desktop.png")
    await accessibility(page, "fixes-policies-axe.json")
    const query = page.getByRole("searchbox", {
      name: "Search tools",
      exact: true,
    })
    await query.fill("*.delete_*")
    await expect(page.getByText("150 tools", { exact: true })).toBeVisible()
    await expect(tools.locator(".data-row")).toHaveCount(50)
    await tools.focus()
    await page.keyboard.press("Control+a")
    await page.keyboard.press("Control+a")
    await expect(
      page.getByRole("heading", { name: "150 selected", exact: true })
    ).toBeVisible()
    await page
      .getByRole("region", { name: "Bulk actions", exact: true })
      .getByRole("button", { name: "Block", exact: true })
      .click()
    await expect(page.getByRole("status")).toContainText("144 tools updated")
    await query.fill("acme_billing.*")
    await expect(page.getByText("300 tools", { exact: true })).toBeVisible()
    await expect(tools.locator(".data-row")).toHaveCount(50)
    await tools.focus()
    await page.keyboard.press("End")
    await expect(tools.locator(".data-row")).toHaveCount(100)
    for (let expected = 150; expected <= 300; expected += 50) {
      await tools.locator(".data-body").evaluate((body) => {
        body.scrollTop = body.scrollHeight
      })
      await expect(tools.locator(".data-row")).toHaveCount(expected)
    }
    await tools.focus()
    await measureLargeListKeys(page, resolve(output, "large-list-latency.json"))
    await expect(
      page.getByRole("button", { name: "Next page", exact: true })
    ).toHaveCount(0)
    expect(
      await tools.evaluate((grid) => getComputedStyle(grid).outlineStyle)
    ).toBe("none")
    await page
      .getByLabel("Tool scope", { exact: true })
      .selectOption("selected")
    await page
      .getByRole("button", { name: "Save tool scope", exact: true })
      .click()
    await expect(page.getByLabel("Tool scope", { exact: true })).toHaveValue(
      "selected"
    )
    await tools.focus()
    await page.keyboard.press("Control+a")
    await page.keyboard.press("Control+a")
    await expect(
      page.getByRole("heading", { name: "300 selected", exact: true })
    ).toBeVisible()
    await page
      .getByRole("region", { name: "Bulk actions", exact: true })
      .getByRole("button", { name: "Include", exact: true })
      .click()
    await expect(page.getByRole("status")).toContainText("300 tools updated")
    await tools.focus()
    await measureLargeListKeys(
      page,
      resolve(output, "large-scoped-list-latency.json")
    )
    await page.goto(`/scripts?workspace=${owned.empty.id}`)
    await expect(
      page.getByText("No saved scripts", { exact: true })
    ).toBeVisible()
    await expect(
      page
        .getByRole("grid", { name: "Scripts", exact: true })
        .getByRole("columnheader")
    ).toHaveCount(0)
    await expect(
      page
        .locator('[data-slot="empty"]')
        .getByText("New script", { exact: true })
    ).toBeVisible()
    await capture(page, "fixes-empty-scripts-desktop.png")
    await page.setViewportSize({ width: 390, height: 844 })
    await noOverflow(page)
    await capture(page, "fixes-empty-scripts-mobile.png")
    await accessibility(page, "fixes-empty-mobile-axe.json")
    await page.goto(`/workspaces?workspace=${workspace.id}&q=acme_billing.*`)
    const select = page.getByRole("button", { name: "Select", exact: true })
    await select.click()
    await tools
      .getByRole("checkbox", {
        name: "Select acme_billing.delete_item_000",
        exact: true,
      })
      .check()
    await tools
      .getByRole("checkbox", {
        name: "Select acme_billing.delete_item_001",
        exact: true,
      })
      .check()
    await expect(
      page.getByRole("heading", { name: "2 selected", exact: true })
    ).toBeVisible()
    await noOverflow(page)
    await capture(page, "fixes-bulk-mobile.png")
    expect(errors).toEqual([])
  } finally {
    await page.evaluate(
      async ({ sourceId, workspace, owned }) => {
        await fetch(`/api/sources/${sourceId}`, { method: "DELETE" })
        await fetch(`/api/tokens/${owned.token.id}`, { method: "DELETE" })
        await fetch(`/api/workspaces/${workspace}/scripts/${owned.script.id}`, {
          method: "DELETE",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ revision: owned.script.revision }),
        })
        await fetch(`/api/workspaces/${owned.empty.id}`, { method: "DELETE" })
      },
      { sourceId, workspace: workspace.id, owned }
    )
  }
})

test("bulk panes: custom sync, revision-safe script deletion and token revocation", async ({
  page,
}) => {
  await page.goto("/login?redirect=%2Fintegrations")
  await expect(
    page.getByRole("heading", {
      name: /^(Create your account|Sign in to your control plane)$/,
    })
  ).toBeVisible()
  const creating = await page
    .getByRole("heading", { name: "Create your account", exact: true })
    .isVisible()
  await page.getByLabel("Email", { exact: true }).fill("ursus95@gmail.com")
  await page.getByLabel("Password", { exact: true }).fill(password)
  await page
    .getByRole("button", {
      name: creating ? "Create account" : "Sign in",
      exact: true,
    })
    .click()
  await expect(page).toHaveURL(`${origin}/integrations`)
  const owned = await page.evaluate(async (url) => {
    const workspace = (
      await fetch("/api/workspaces").then((r) => r.json())
    ).find((w: { allSources: boolean }) => w.allSources).id
    const create = async (path: string, body: object) => {
      const response = await fetch(path, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body),
      })
      if (!response.ok) throw new Error(`Fixture creation failed: ${path}`)
      return response.json()
    }
    const sources = [],
      scripts = [],
      tokens = []
    for (const suffix of ["a", "b"]) {
      sources.push(
        await create("/api/sources", {
          slug: `bulk_connector_${suffix}`,
          name: `Bulk connector ${suffix}`,
          icon: "plug",
          kind: "mcp_http",
          url,
          requiresAccount: false,
        })
      )
      scripts.push(
        await create(`/api/workspaces/${workspace}/scripts`, {
          name: `bulk_script_${suffix}`,
          description: "Bulk deletion fixture",
          code: "return 1",
        })
      )
      tokens.push(
        await create("/api/tokens", {
          name: `bulk_revoke_${suffix}`,
          workspaceId: workspace,
        })
      )
    }
    return { workspace, sources, scripts, tokens }
  }, fixtureUrl)
  try {
    await page.goto("/integrations?q=Bulk*")
    const integrations = page.getByRole("grid", {
      name: "Integrations",
      exact: true,
    })
    await expect(integrations.locator(".data-row")).toHaveCount(2)
    await integrations.focus()
    await page.keyboard.press("Control+a")
    await page.keyboard.press("Control+a")
    await expect(
      page.getByRole("heading", { name: "2 selected", exact: true })
    ).toBeVisible()
    await page
      .getByRole("button", { name: "Sync connectors", exact: true })
      .click()
    await expect(page.getByRole("status")).toContainText(
      "Synchronization requested for 2 connectors"
    )
    await page.goto(`/scripts?workspace=${owned.workspace}&q=bulk_script_*`)
    const scripts = page.getByRole("grid", { name: "Scripts", exact: true })
    await expect(scripts.locator(".data-row")).toHaveCount(2)
    await scripts.focus()
    await page.keyboard.press("Control+a")
    await page.keyboard.press("Control+a")
    await page.keyboard.press("Delete")
    await expect(
      page.getByRole("button", { name: /Keep selection/ })
    ).toBeFocused()
    await capture(page, "fixes-script-confirm-desktop.png")
    await accessibility(page, "fixes-script-confirm-axe.json")
    await page.keyboard.press("Tab")
    await expect(
      page.getByRole("button", { name: "Delete scripts", exact: true })
    ).toBeFocused()
    await page.keyboard.press("Escape")
    await expect(
      page.getByRole("heading", { name: "2 selected", exact: true })
    ).toBeVisible()
    await expect(
      page.getByRole("button", { name: /Keep selection/ })
    ).toHaveCount(0)
    await page
      .getByRole("button", { name: "Delete scripts", exact: true })
      .click()
    await page
      .getByRole("button", { name: "Delete scripts", exact: true })
      .click()
    await expect(scripts.locator(".data-row")).toHaveCount(0)
    await expect(page.getByRole("status")).toContainText("2 scripts deleted")
    await page.goto("/tokens?q=bulk_revoke_*")
    const tokens = page.getByRole("grid", {
      name: "Agents with access",
      exact: true,
    })
    await expect(tokens.locator(".data-row")).toHaveCount(2)
    await tokens.focus()
    await page.keyboard.press("Control+a")
    await page.keyboard.press("Control+a")
    await page
      .getByRole("button", { name: "Revoke tokens", exact: true })
      .click()
    await expect(
      page.getByRole("button", { name: /Keep selection/ })
    ).toBeFocused()
    await capture(page, "fixes-token-confirm-desktop.png")
    await page
      .getByRole("button", { name: "Revoke tokens", exact: true })
      .click()
    await expect(page.getByRole("status")).toContainText(
      "2 access tokens revoked"
    )
    const values = await page.evaluate(() =>
      fetch("/api/tokens").then((r) => r.json())
    )
    for (const token of owned.tokens)
      expect(
        values.find((value: { id: string }) => value.id === token.id)?.revoked
      ).toBe(true)
    const denied = await page.request.post(`${loopback}/mcp`, {
      headers: {
        host: hostname,
        Authorization: `Bearer ${owned.tokens[0].token}`,
        Accept: "application/json, text/event-stream",
        "MCP-Protocol-Version": "2025-11-25",
      },
      data: { jsonrpc: "2.0", id: 1, method: "tools/list", params: {} },
    })
    expect(denied.status()).toBe(401)
  } finally {
    await page.evaluate(async (owned) => {
      for (const source of owned.sources)
        await fetch(`/api/sources/${source.id}`, { method: "DELETE" })
      for (const script of owned.scripts)
        await fetch(`/api/workspaces/${owned.workspace}/scripts/${script.id}`, {
          method: "DELETE",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ revision: script.revision }),
        })
      for (const token of owned.tokens)
        await fetch(`/api/tokens/${token.id}`, { method: "DELETE" })
    }, owned)
  }
})
