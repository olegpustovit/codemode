const relative = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" })
const units = [
  ["day", 86_400_000],
  ["hour", 3_600_000],
  ["minute", 60_000],
  ["second", 1_000],
] as const

export function relativeTime(timestamp: string, now = Date.now()) {
  const delta = Date.parse(timestamp) - now
  if (!Number.isFinite(delta)) return "—"
  for (const [unit, ms] of units) {
    if (Math.abs(delta) >= ms || unit === "second") {
      return relative.format(Math.trunc(delta / ms), unit)
    }
  }
}
