const relative = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" })
const monthDay = new Intl.DateTimeFormat(undefined, {
  month: "short",
  day: "numeric",
})
const clock = new Intl.DateTimeFormat(undefined, {
  hour: "2-digit",
  minute: "2-digit",
  hourCycle: "h23",
})
const units = [
  ["day", 86_400_000],
  ["hour", 3_600_000],
  ["minute", 60_000],
  ["second", 1_000],
] as const

export function relativeTime(timestamp: string, now = Date.now()) {
  const date = new Date(timestamp)
  const delta = date.getTime() - now
  if (!Number.isFinite(delta)) return "—"
  if (Math.abs(delta) >= 7 * units[0][1]) {
    const year =
      date.getFullYear() === new Date(now).getFullYear()
        ? ""
        : ` ${date.getFullYear()}`
    return `${monthDay.format(date)}${year} ${clock.format(date)}`
  }
  for (const [unit, ms] of units) {
    if (Math.abs(delta) >= ms || unit === "second") {
      return relative.format(Math.trunc(delta / ms), unit)
    }
  }
}
