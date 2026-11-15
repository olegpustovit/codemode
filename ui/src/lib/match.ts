type Range = { start: number; end: number }

function termRanges(term: string, value: string): Range[] | null {
  const lower = value.toLowerCase()
  if (!/[*?]/.test(term)) {
    const start = lower.indexOf(term.toLowerCase())
    if (start < 0) return null
    const end = start + term.toLowerCase().length
    if (lower.length === value.length) return [{ start, end }]
    let offset = 0
    const spans = Array.from(value).flatMap((character) => {
      const span = { start: offset, end: offset + character.length }
      offset += character.length
      return Array.from({ length: character.toLowerCase().length }, () => span)
    })
    return [{ start: spans[start].start, end: spans[end - 1].end }]
  }
  const pattern = Array.from(term, (character) => character.toLowerCase()),
    text = Array.from(value, (character) => character.toLowerCase())
  let p = 0,
    i = 0,
    star = -1,
    resume = 0
  const positions: (number | undefined)[] = []
  // Retry only the most recent star; never use exponential regex backtracking.
  while (i < text.length) {
    if (pattern[p] === "?" || pattern[p] === text[i]) {
      positions[p] = pattern[p] === "?" ? undefined : i
      p++
      i++
    } else if (pattern[p] === "*") {
      star = p++
      resume = i
      positions[star] = undefined
    } else if (star >= 0) {
      p = star + 1
      i = ++resume
      positions.length = p
    } else return null
  }
  while (pattern[p] === "*") p++
  if (p !== pattern.length) return null
  const offsets = [0]
  for (const character of Array.from(value))
    offsets.push(offsets[offsets.length - 1] + character.length)
  return positions
    .filter((position) => position !== undefined)
    .map((position) => ({
      start: offsets[position],
      end: offsets[position + 1],
    }))
}
export function matches(query: string, ...values: string[]) {
  return query
    .trim()
    .split(/\s+/)
    .filter(Boolean)
    .every((term) => values.some((value) => termRanges(term, value) !== null))
}
export function matchRanges(query: string, value: string): Range[] {
  const ranges = query
    .trim()
    .split(/\s+/)
    .filter(Boolean)
    .flatMap((term) => termRanges(term, value) ?? [])
    .sort((a, b) => a.start - b.start)
  const merged: Range[] = []
  for (const range of ranges) {
    const previous = merged[merged.length - 1]
    if (previous && range.start <= previous.end)
      previous.end = Math.max(previous.end, range.end)
    else merged.push({ ...range })
  }
  return merged
}
