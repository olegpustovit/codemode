import { Fragment } from "react"
import { matchRanges } from "@/lib/match"
export function MatchText({
  query,
  children,
}: {
  query: string
  children: string
}) {
  const ranges = matchRanges(query, children)
  return (
    <>
      {ranges.map(({ start, end }, index) => {
        return (
          <Fragment key={start}>
            {children.slice(index ? ranges[index - 1].end : 0, start)}
            <mark>{children.slice(start, end)}</mark>
          </Fragment>
        )
      })}
      {children.slice(ranges.at(-1)?.end ?? 0)}
    </>
  )
}
export function SearchHint({ hoverOnly = false }: { hoverOnly?: boolean }) {
  return (
    <span
      className="search-hint"
      id="list-search-hint"
      data-hover-only={hoverOnly || undefined}
    >
      * any characters · ? one character · spaces match all terms
    </span>
  )
}
