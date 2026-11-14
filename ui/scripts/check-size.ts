import { readFileSync, readdirSync, existsSync } from "node:fs"
import { gzipSync } from "node:zlib"
import { fileURLToPath } from "node:url"
export function verifyBuild(root: string) {
  const html = readFileSync(`${root}/index.html`, "utf8")
  const initial = [
    ...html.matchAll(/(?:src|href)="\/(assets\/[^"?#]+\.(?:js|css))"/g),
  ].map((match) => match[1])
  const size = (file: string) =>
    gzipSync(readFileSync(`${root}/${file}`)).length
  const total = (suffix: string) =>
    initial
      .filter((file) => file.endsWith(suffix))
      .reduce((sum, file) => sum + size(file), 0)
  if (!initial.some((file) => file.endsWith(".js")))
    throw new Error("No entry script found")
  if (total(".js") > 170000 || total(".css") > 30000)
    throw new Error(
      `Initial bundle exceeds budget: JS ${total(".js")} B, CSS ${total(".css")} B gzip`
    )
  for (const file of readdirSync(`${root}/assets`))
    if (
      file.endsWith(".js") &&
      !initial.includes(`assets/${file}`) &&
      size(`assets/${file}`) > 100000
    )
      throw new Error(`Lazy chunk ${file} exceeds 100 KB gzip`)
  for (const match of html.matchAll(/(?:src|href)="(\/[^"?#]+)"/g))
    if (!existsSync(`${root}${match[1]}`))
      throw new Error(`Missing linked asset ${match[1]}`)
  for (const file of readdirSync(`${root}/assets`).filter((file) =>
    file.endsWith(".css")
  )) {
    const css = readFileSync(`${root}/assets/${file}`, "utf8")
    for (const match of css.matchAll(
      /url\(["']?(?!data:)([^)'"?#]+)["']?\)/g
    )) {
      const target = match[1].startsWith("/")
        ? `${root}${match[1]}`
        : `${root}/assets/${match[1]}`
      if (!existsSync(target)) throw new Error(`Missing CSS asset ${target}`)
    }
  }
  console.log(
    `Bundle: initial JS ${total(".js")} B, CSS ${total(".css")} B gzip; lazy chunks ≤100 KB`
  )
  const manifest = JSON.parse(readFileSync(`${root}/site.webmanifest`, "utf8"))
  for (const icon of manifest.icons)
    if (!existsSync(`${root}${icon.src}`))
      throw new Error(`Missing manifest icon ${icon.src}`)
}
if (import.meta.main)
  verifyBuild(fileURLToPath(new URL("../dist/", import.meta.url)))
