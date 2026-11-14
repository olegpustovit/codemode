import { defineConfig } from "@playwright/test"
import { execFileSync } from "node:child_process"

export default defineConfig({
  testDir: "./e2e",
  workers: 1,
  retries: 0,
  timeout: 90_000,
  globalTimeout: 180_000,
  reporter: "list",
  use: {
    ignoreHTTPSErrors: true,
    actionTimeout: 10_000,
    baseURL: "https://codemode.microapps.space",
    viewport: { width: 1440, height: 1000 },
    launchOptions: {
      executablePath:
        process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE ??
        execFileSync("which", ["chromium"], { encoding: "utf8" }).trim(),
    },
  },
})
