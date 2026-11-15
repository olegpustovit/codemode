import { spawnSync } from "node:child_process"
import { resolve } from "node:path"
const root = resolve(import.meta.dirname, "..")
const args = process.argv.slice(2)
// Keep each suite under its unchanged 180-second deadline, with fresh tenant fixtures.
const standalone =
  "integrations:|OAuth apps:|sessions:|control-plane fixes:|bulk panes:"
const batches = args.length
  ? [args]
  : [
      ["--grep-invert", standalone],
      ["--grep", standalone],
    ]
for (const batch of batches) {
  const result = spawnSync(
    resolve(root, "node_modules/.bin/playwright"),
    ["test", ...batch],
    { cwd: root, stdio: "inherit" }
  )
  if (result.error) throw result.error
  if (result.status !== 0) process.exit(result.status ?? 1)
}
