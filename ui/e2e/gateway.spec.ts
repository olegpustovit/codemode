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
            tools: [
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
    CODEMODE_SNAPSHOT_KEY: randomBytes(32).toString("hex"),
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
    page.getByRole("heading", { name: "Sign in to your control plane" })
  ).toBeVisible()
  await page.getByLabel("Email", { exact: true }).fill("ursus95@gmail.com")
  await page.getByLabel("Password", { exact: true }).fill(password)
  await page.getByRole("button", { name: "Sign in", exact: true }).click()
  await expect(page).toHaveURL(`${origin}${path}`)
}
async function capture(page: Page, file: string) {
  await page.evaluate(() => document.fonts.ready.then(() => undefined))
  await page.evaluate(() => scrollTo(0, 0))
  await page.screenshot({
    path: resolve(output, file),
    fullPage: true,
    animations: "disabled",
  })
}
async function noOverflow(page: Page) {
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth
    )
  ).toBe(true)
}
async function contrast(button: Locator) {
  return button.evaluate((element) => {
    const canvas = document.createElement("canvas")
    canvas.width = canvas.height = 1
    const context = canvas.getContext("2d")
    if (!context) throw new Error("Canvas unavailable for contrast measurement")
    const style = getComputedStyle(element)
    context.fillStyle = "#fff"
    context.fillRect(0, 0, 1, 1)
    context.fillStyle = style.backgroundColor
    context.fillRect(0, 0, 1, 1)
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
  })
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
  await page.getByLabel("Email", { exact: true }).fill("outsider@example.com")
  await page.getByLabel("Password", { exact: true }).fill(password)
  await page
    .getByRole("button", { name: "Create account", exact: true })
    .click()
  await expect(page.getByRole("alert")).toContainText("registration is limited")
  await page.getByLabel("Email", { exact: true }).fill("ursus95@gmail.com")
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
  await page.getByRole("button", { name: "Sign out", exact: true }).click()
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
  await expect(page.getByText("Connect your first source")).toBeVisible()
  await capture(page, "desktop-empty.png")
  await page
    .getByRole("link", { name: "Add source", exact: true })
    .first()
    .click()
  await page.getByLabel("Source name", { exact: true }).fill("browser-fixture")
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
  await page.setViewportSize({width:390,height:844})
  await noOverflow(page)
  await capture(page,"workspace-editor-mobile.png")
  await page.setViewportSize({width:1440,height:1000})
  await page.goto("/tokens")
  await page.getByLabel("Name", { exact: true }).fill("Browser agent")
  await page.getByLabel("Workspace", { exact: true }).selectOption(workspaceId)
  await page.getByRole("button", { name: "Create token", exact: true }).click()
  const token = await page.getByLabel("New access token").inputValue()
  expect(token).toMatch(/^cm_/)
  await page.getByRole("button", { name: "I saved it" }).click()
  await expect(page.getByLabel("New access token")).toHaveCount(0)
  await page.reload()
  await expect(page.getByLabel("New access token")).toHaveCount(0)
  await expect(page.getByText("Browser agent", { exact: true })).toBeVisible()
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
  await page.goto("/sources")
  await expect(
    page.getByRole("link", { name: "browser-fixture", exact: true })
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
      (font) => font.isCustomFont && font.familyName.includes("Geist")
    )
  ).toBe(true)
  await cdp.detach()
  await capture(page, "desktop-sources.png")
  await page.goto("/executions")
  await expect(
    page.getByRole("heading", { name: "Executions", exact: true })
  ).toBeVisible()
  await capture(page, "desktop-executions.png")
  await page
    .getByRole("link", {
      name: `Execution ${execution.executionId.slice(0, 8)}`,
      exact: true,
    })
    .click()
  await expect(
    page.getByRole("heading", { name: "Python code", exact: true })
  ).toBeVisible()
  await expect(page.getByRole("heading", { name: /echo/ })).toBeVisible()
  await expect(
    page.locator("pre").filter({ hasText: '"value": "fixture-value"' })
  ).toBeVisible()
  await page.reload()
  await expect(
    page.getByRole("heading", { name: "Python code", exact: true })
  ).toBeVisible()
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
  await expect(page.getByRole("button", { name: "Authorize workspace", exact: true })).toBeDisabled()
  await page.getByLabel("Workspace", {exact:true}).selectOption(workspaceId)
  await expect(page.getByRole("alert")).toContainText("browser-fixture")
  await capture(page, "desktop-consent.png")
  await page.setViewportSize({width:390,height:844})
  await noOverflow(page)
  await capture(page,"workspace-consent-mobile.png")
  await page.setViewportSize({width:1440,height:1000})
  await page.getByRole("button", { name: "Deny", exact: true }).click()
  await expect(page).toHaveURL(
    /client\.example\/callback\?.*error=access_denied/
  )
  expect(new URL(page.url()).searchParams.get("state")).toBe("browser-state")
  await page.goto(`${origin}/oauth/authorize?${query}`)
  await page.getByLabel("Workspace", {exact:true}).selectOption(workspaceId)
  await page
    .getByRole("button", { name: "Authorize workspace", exact: true })
    .click()
  await expect(page).toHaveURL(/client\.example\/callback\?.*code=/)
  expect(new URL(page.url()).searchParams.get("iss")).toBe(origin)
  await page.goto(`${origin}/workspaces/${workspaceId}`)
  await page
    .getByRole("button", { name: "Delete workspace", exact: true })
    .click()
  await page
    .getByRole("alertdialog")
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
  await page.getByRole("button", { name: "Sign out", exact: true }).click()
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
    page.getByRole("heading", { name: "Sources", exact: true })
  ).toBeVisible()
  await capture(page, "mobile-sources.png")
  await noOverflow(page)
  await page.getByRole("button", { name: "Toggle Sidebar" }).click()
  await expect(page.getByRole("dialog")).toHaveCSS("opacity", "1")
  await capture(page, "mobile-navigation.png")
  await page.getByRole("link", { name: "Accounts", exact: true }).click()
  await expect(
    page.getByRole("heading", { name: "Linked accounts", exact: true })
  ).toBeVisible()
  await expect(page.getByRole("dialog")).toHaveCount(0)
  await page.goto("/sources?q=browser")
  await page.reload()
  await expect(page.getByLabel("Find a source")).toHaveValue("browser")
  await page.goto("/sources/new")
  await page.getByLabel("Source name", { exact: true }).fill("x".repeat(64))
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
  await page.getByLabel("Source name", { exact: true }).fill("browser-openapi")
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
      body: JSON.stringify({ name: "OpenAPI browser check", workspaceId: (await fetch("/api/workspaces").then((r) => r.json())).find((w: {allSources:boolean})=>w.allSources).id }),
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
      page.getByRole("heading", {
        name: `1. ${source.pythonName}.listItems`,
        exact: true,
      })
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
      page.getByRole("heading", { name: "Tool policies", exact: true })
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
    const policyName = `Policy for ${source.pythonName}.listItems`
    const saveName = `Save policy for ${source.pythonName}.listItems`
    await page.getByLabel(policyName, { exact: true }).selectOption("block")
    await update(`/api/workspaces/${main.id}/policies`, () =>
      page.getByRole("button", { name: saveName, exact: true }).click()
    )
    await expect(
      page.getByRole("button", { name: saveName, exact: true })
    ).toBeDisabled()
    await page.reload()
    await expect(page.getByLabel(policyName, { exact: true })).toHaveValue(
      "block"
    )
    const blocked = await invoke("search", { query: "listItems" })
    expect(JSON.parse(blocked.content[0].text).tools).toHaveLength(0)
    await capture(page, "tool-policies.png")
    await page.getByLabel(policyName, { exact: true }).selectOption("default")
    await update(`/api/workspaces/${main.id}/policies`, () =>
      page.getByRole("button", { name: saveName, exact: true }).click()
    )
    await expect(
      page.getByRole("button", { name: saveName, exact: true })
    ).toBeDisabled()
    await page
      .getByLabel("Tool scope", { exact: true })
      .selectOption("selected")
    await update(`/api/workspaces/${main.id}/scope`, () =>
      page.getByRole("button", { name: "Save tool scope", exact: true }).click()
    )
    await expect(
      page.getByRole("button", { name: "Save tool scope", exact: true })
    ).toBeDisabled()
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
    const paused = JSON.parse(
      (
        await invoke("execute", {
          code: `print("before approval")\nreturn await asyncio.gather(tools.${source.pythonName}.listItems(), tools.${source.pythonName}.deleteItems())`,
        })
      ).content[0].text
    )
    expect(paused.status).toBe("paused")
    expect(destructiveCalls).toBe(0)
    expect(readCalls).toBe(readsBefore)
    const forged = await invoke("resume", {
      executionId: paused.executionId,
      allow: true,
    })
    expect(forged.isError).toBe(true)
    expect(
      JSON.parse(
        (await invoke("resume", { executionId: paused.executionId })).content[0]
          .text
      ).status
    ).toBe("paused")
    await restartGateway()
    expect(
      JSON.parse(
        (await invoke("resume", { executionId: paused.executionId })).content[0]
          .text
      ).status
    ).toBe("paused")
    await page.goto(paused.approvalUrl)
    await expect(
      page.getByText("This batch is waiting", { exact: true })
    ).toBeVisible()
    await expect(
      page.getByRole("heading", {
        name: `${source.pythonName}.deleteItems`,
        exact: true,
      })
    ).toBeVisible()
    await capture(page, "approval-desktop.png")
    await page.setViewportSize({ width: 390, height: 844 })
    await noOverflow(page)
    await capture(page, "approval-mobile.png")
    await page
      .getByRole("button", {
        name: `Allow ${source.pythonName}.deleteItems`,
        exact: true,
      })
      .click()
    await expect(
      page.getByRole("button", {
        name: `Allow ${source.pythonName}.deleteItems`,
        exact: true,
      })
    ).toHaveCount(0)
    expect(destructiveCalls).toBe(0)
    expect(
      JSON.parse(
        (await invoke("resume", { executionId: paused.executionId })).content[0]
          .text
      ).status
    ).toBe("paused")
    await page
      .getByRole("button", {
        name: `Allow ${source.pythonName}.listItems`,
        exact: true,
      })
      .click()
    await expect(
      page.getByText("Decisions saved", { exact: true })
    ).toBeVisible()
    const resumed = JSON.parse(
      (await invoke("resume", { executionId: paused.executionId })).content[0]
        .text
    )
    expect(resumed.status).toBe("completed")
    expect(resumed.result).toEqual([["item-one"], "deleted"])
    expect(resumed.output).toBe("before approval\n")
    expect(destructiveCalls).toBe(1)
    expect(readCalls).toBe(readsBefore + 1)
    expect(
      (await invoke("resume", { executionId: paused.executionId })).isError
    ).toBe(true)
    await page.reload()
    await expect(
      page.getByText("This approval is no longer available", { exact: true })
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
    await page.getByLabel("Source name", { exact: true }).fill("unsaved-name")
    await page.evaluate((path) => {
      history.pushState({}, "", path)
      dispatchEvent(new PopStateEvent("popstate"))
    }, `/sources/${others[0].id}`)
    await expect(page.getByLabel("Source name", { exact: true })).toHaveValue(
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

test("GraphQL: add a source, link a credential, scope its tools and approve a mutation", async ({
  page,
}) => {
  await login(page, "/sources/new")
  await page.getByLabel("Source name", { exact: true }).fill("browser-graphql")
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
        new URL(r.url()).pathname === `/api/workspaces/${main.id}/scope/tools` &&
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
      body: JSON.stringify({ name: "GraphQL browser check", workspaceId: (await fetch("/api/workspaces").then((r) => r.json())).find((w: {allSources:boolean})=>w.allSources).id }),
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
    const paused = JSON.parse(
      (
        await invoke("execute", {
          code: `return await tools.${source.pythonName}.mutation(query='mutation { deleteItem }')`,
        })
      ).content[0].text
    )
    expect(paused.status).toBe("paused")
    expect(graphqlMutations).toBe(before)
    await page.goto(paused.approvalUrl)
    await expect(
      page.getByRole("heading", {
        name: `${source.pythonName}.mutation`,
        exact: true,
      })
    ).toBeVisible()
    await page
      .getByRole("button", {
        name: `Allow ${source.pythonName}.mutation`,
        exact: true,
      })
      .click()
    await expect(
      page.getByText("Decisions saved", { exact: true })
    ).toBeVisible()
    const resumed = JSON.parse(
      (await invoke("resume", { executionId: paused.executionId })).content[0]
        .text
    )
    expect(resumed.status).toBe("completed")
    expect(resumed.result.kind).toBe("mutation")
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
      body: JSON.stringify({ name: "Saved script browser check", workspaceId: (await fetch("/api/workspaces").then((r) => r.json())).find((w: {allSources:boolean})=>w.allSources).id }),
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
      ).toEqual(["execute", "resume", "search"])
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
  await page
    .getByRole("alertdialog")
    .getByRole("button", { name: "Delete script", exact: true })
    .click()
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
      method: "PUT", headers: { "Content-Type": "application/json" }, body: JSON.stringify({allTools:true}),
    })
    if (scope.status !== 204) throw new Error("Could not prepare owner endpoint scope")
    const token = await fetch("/api/tokens", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ name: "Tenant owner check", workspaceId: main.id }),
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
  await page.getByRole("button", { name: "Sign out", exact: true }).click()
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
  await expect(page).toHaveURL(`${origin}/sources`)
  await expect(page.getByText("Connect your first source")).toBeVisible()
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
    page.getByRole("link", { name: "browser-fixture", exact: true })
  ).toBeVisible()
  await capture(page, "tenant-secondary.png")
  await restartGateway()
  expect(
    (await rpc(secondary.token.token, "search", { query: "" })).value.tools
  ).toHaveLength(1)
  await page.getByRole("button", { name: "Sign out", exact: true }).click()
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

test("workspace bookmarks, preset connection and separate administration authorization", async ({ page }) => {
  await login(page, "/workspaces")
  await page.goto("/endpoints")
  await expect(page).toHaveURL(`${origin}/workspaces`)
  await page.goto("/sources")
  await expect(page.getByRole("heading", { name: "Popular MCP servers" })).toBeVisible()
  await capture(page, "workspace-source-presets-desktop.png")
  await page.setViewportSize({ width: 390, height: 844 })
  await noOverflow(page)
  await capture(page, "workspace-source-presets-mobile.png")
  await page.getByRole("button", { name: "Add Notion", exact: true }).click()
  await expect(page).toHaveURL(/\/sources\/[0-9a-f-]+\/accounts$/)
  const sourceId = new URL(page.url()).pathname.split("/").at(-2)!
  const preset = await page.evaluate((id) => fetch(`/api/sources/${id}`).then((r) => r.json()), sourceId)
  expect(preset.url).toBe("https://mcp.notion.com/mcp")
  expect(preset.requiresAccount).toBe(true)
  await expect(page.getByRole("button", { name: "Connect with OAuth", exact: true })).toBeVisible()
  // No provider sign-in or external consent is performed by this test.
  await page.evaluate((id) => fetch(`/api/sources/${id}`, {method:"DELETE"}), sourceId)
  await page.setViewportSize({ width: 1440, height: 1000 })
  await page.goto("/tokens")
  await page.getByLabel("Access", {exact:true}).selectOption("admin")
  await expect(page.getByLabel("Workspace", {exact:true})).toHaveCount(0)
  await page.getByLabel("Name", {exact:true}).fill("Management browser check")
  await page.getByRole("button", {name:"Create token",exact:true}).click()
  const token = await page.getByLabel("New access token").inputValue()
  const rpc = async (resource:string, method:string, params:object) => {
    const response = await page.request.post(`${loopback}${resource}`, {
      headers: {host:hostname, Authorization:`Bearer ${token}`, Accept:"application/json, text/event-stream", "MCP-Protocol-Version":"2025-11-25"},
      data: {jsonrpc:"2.0", id:1, method, params},
    })
    return response
  }
  expect((await rpc("/mcp","tools/list",{})).status()).toBe(401)
  const tools = await (await rpc("/mcp-admin","tools/list",{})).json()
  expect(tools.result.tools).toHaveLength(9)
  const modern=await page.request.post(`${loopback}/mcp-admin`, {
    headers:{host:hostname,Authorization:`Bearer ${token}`,Accept:"application/json, text/event-stream","MCP-Protocol-Version":"2026-07-28","Mcp-Method":"tools/list"},
    data:{jsonrpc:"2.0",id:1,method:"tools/list",params:{_meta:{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{},"io.modelcontextprotocol/clientInfo":{name:"management-browser",version:"1"}}}},
  })
  expect(modern.status()).toBe(200)
  expect((await modern.json()).result.tools).toHaveLength(9)
  const workspaces = await (await rpc("/mcp-admin","tools/call",{name:"list_workspaces",arguments:{}})).json()
  expect(JSON.parse(workspaces.result.content[0].text).some((w: {allSources:boolean})=>w.allSources)).toBe(true)
  await page.getByRole("button", {name:"I saved it",exact:true}).click()
  await expect(page.getByRole("cell",{name:"Administration /mcp-admin",exact:true})).toBeVisible()
  const registration = await page.evaluate(() => fetch("/oauth/register",{method:"POST", headers:{"Content-Type":"application/json"},body:JSON.stringify({client_name:"Workspace manager",redirect_uris:["https://client.example/callback"],token_endpoint_auth_method:"none"})}).then((r)=>r.json()))
  const query=new URLSearchParams({response_type:"code",client_id:registration.client_id,redirect_uri:"https://client.example/callback",resource:`${origin}/mcp-admin`,scope:"admin",code_challenge:createHash("sha256").update("browser-verifier-123456789012345678901234567890123456789").digest("base64url"),code_challenge_method:"S256"})
  await page.goto(`/oauth/authorize?${query}`)
  await expect(page.getByRole("heading",{name:"Authorize Workspace manager?"})).toBeVisible()
  await expect(page.getByLabel("Workspace",{exact:true})).toHaveCount(0)
  await expect(page.getByRole("alert")).toContainText("create, change and delete your workspaces")
  await capture(page,"workspace-admin-consent-desktop.png")
  await page.setViewportSize({width:390,height:844})
  await noOverflow(page)
  await capture(page,"workspace-admin-consent-mobile.png")
  await page.getByRole("button",{name:"Authorize administration",exact:true}).click()
  await expect(page).toHaveURL(/client\.example\/callback\?.*code=/)
})
