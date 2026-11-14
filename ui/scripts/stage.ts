import { createHash } from "node:crypto"
import {
  cpSync,
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  renameSync,
  rmSync,
  writeFileSync,
} from "node:fs"
import { resolve, relative } from "node:path"
import { execFileSync } from "node:child_process"
import { verifyBuild } from "./check-size"

const ui = resolve(import.meta.dirname, "..")
const staged = resolve(ui, "../crates/gateway/ui-dist")
const roots = [
  "src",
  "public",
  "scripts",
  "index.html",
  "vite.config.ts",
  "tsconfig.json",
  "components.json",
  "package.json",
  "bun.lock",
]
function files(root: string, path = root): string[] {
  return readdirSync(path, { withFileTypes: true })
    .flatMap((entry) => {
      if (entry.name.startsWith(".")) return []
      const full = resolve(path, entry.name)
      if (entry.isDirectory()) return files(root, full)
      if (!entry.isFile()) throw new Error(`Unexpected non-file ${full}`)
      return [relative(root, full)]
    })
    .sort()
}
function digest(root: string, paths: string[]) {
  const hash = createHash("sha256")
  for (const path of paths.sort())
    hash.update(
      `${path}\0${createHash("sha256")
        .update(readFileSync(resolve(root, path)))
        .digest("hex")}\n`
    )
  return hash.digest("hex")
}
function inputs() {
  return digest(
    ui,
    roots.flatMap((path) =>
      path.includes(".")
        ? [path]
        : files(resolve(ui, path)).map((file) => `${path}/${file}`)
    )
  )
}
function fresh() {
  if (
    !existsSync(`${staged}/.fingerprint`) ||
    !existsSync(`${staged}/index.html`)
  )
    return false
  const [source, output] = readFileSync(`${staged}/.fingerprint`, "utf8")
    .trim()
    .split("\n")
  return source === inputs() && output === digest(staged, files(staged))
}
if (process.argv.slice(2).some((arg) => arg !== "--check"))
  throw new Error("Usage: bun scripts/stage.ts [--check]")
if (fresh()) {
  verifyBuild(staged)
  console.log("Embedded UI is current; staged files unchanged.")
} else {
  if (process.argv.includes("--check"))
    throw new Error(
      "Embedded UI is missing, stale or modified. Run bun run stage."
    )
  const before = inputs()
  execFileSync(process.execPath, ["run", "build"], {
    cwd: ui,
    stdio: "inherit",
  })
  verifyBuild(`${ui}/dist`)
  if (before !== inputs())
    throw new Error("UI inputs changed during the build; rerun staging.")
  const temporary = `${staged}.new`
  const previous = `${staged}.previous`
  rmSync(temporary, { recursive: true, force: true })
  cpSync(`${ui}/dist`, temporary, { recursive: true })
  writeFileSync(`${temporary}/.gitkeep`, "")
  writeFileSync(
    `${temporary}/.fingerprint`,
    `${before}\n${digest(temporary, files(temporary))}\n`
  )
  rmSync(previous, { recursive: true, force: true })
  mkdirSync(resolve(staged, ".."), { recursive: true })
  if (existsSync(staged)) renameSync(staged, previous)
  try {
    renameSync(temporary, staged)
  } catch (error) {
    if (existsSync(previous)) renameSync(previous, staged)
    throw error
  }
  rmSync(previous, { recursive: true, force: true })
  console.log(`Staged verified UI ${before.slice(0, 12)}.`)
}
