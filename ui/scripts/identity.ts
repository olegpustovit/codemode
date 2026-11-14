import { mkdirSync, writeFileSync } from "node:fs"
import { execFileSync } from "node:child_process"
import { fileURLToPath } from "node:url"
const root = fileURLToPath(new URL("../public/", import.meta.url))
mkdirSync(root, { recursive: true })
const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><rect width="64" height="64" rx="14" fill="#1b2c43"/><g fill="none" stroke="#fff" stroke-width="4" stroke-linecap="round" stroke-linejoin="round"><path d="M16 17 32 32 48 17M32 32v15"/><circle cx="16" cy="17" r="5" fill="#1b2c43"/><circle cx="48" cy="17" r="5" fill="#1b2c43"/><circle cx="32" cy="47" r="5" fill="#1b2c43"/></g></svg>`
writeFileSync(`${root}/icon.svg`, svg)
for (const size of [16, 32, 180, 192, 512])
  execFileSync("convert", [
    "-background",
    "none",
    `${root}/icon.svg`,
    "-resize",
    `${size}x${size}`,
    "-set",
    "comment",
    "Authored geometric mark; source ui/scripts/identity.ts, derived from public/icon.svg.",
    "-set",
    "impeccable:prompt",
    "Origin: authored connection mark in ui/scripts/identity.ts; rendered from ui/public/icon.svg with ImageMagick.",
    `${root}/icon-${size}.png`,
  ])
execFileSync("convert", [
  `${root}/icon-16.png`,
  `${root}/icon-32.png`,
  `${root}/favicon.ico`,
])
execFileSync("convert", [
  `${root}/icon-512.png`,
  "-resize",
  "80%",
  "-background",
  "#1b2c43",
  "-gravity",
  "center",
  "-extent",
  "512x512",
  "-alpha",
  "remove",
  "-set",
  "comment",
  "Authored geometric mark; maskable variant from ui/scripts/identity.ts.",
  "-set",
  "impeccable:prompt",
  "Origin: maskable variant of the authored connection mark in ui/scripts/identity.ts, derived from ui/public/icon.svg with ImageMagick.",
  `${root}/icon-maskable-512.png`,
])
writeFileSync(
  `${root}/site.webmanifest`,
  JSON.stringify(
    {
      name: "Codemode",
      short_name: "Codemode",
      start_url: "/sources",
      display: "standalone",
      background_color: "#ffffff",
      theme_color: "#1b2c43",
      icons: [
        { src: "/icon-192.png", sizes: "192x192", type: "image/png" },
        {
          src: "/icon-512.png",
          sizes: "512x512",
          type: "image/png",
          purpose: "any",
        },
        {
          src: "/icon-maskable-512.png",
          sizes: "512x512",
          type: "image/png",
          purpose: "maskable",
        },
      ],
    },
    null,
    2
  )
)
