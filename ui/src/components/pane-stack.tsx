import { useLayoutEffect, useRef } from "react"
import type { ReactNode } from "react"
export function PaneStack({
  list,
  detail,
  open,
  id,
  label = "Integration",
  execution,
}: {
  list: ReactNode
  detail: ReactNode
  open: boolean
  id: string
  label?: string
  execution?: ReactNode
}) {
  const root = useRef<HTMLDivElement>(null)
  const previous = useRef(0)
  const depth = execution ? 2 : open ? 1 : 0
  useLayoutEffect(() => {
    const changed = previous.current !== depth
    previous.current = depth
    if (changed && matchMedia("(max-width: 1000px)").matches) {
      const frame = requestAnimationFrame(() => {
        root.current
          ?.querySelector<HTMLElement>(
            depth === 2
              ? ".execution-pane button"
              : depth === 1
                ? ".detail-pane button"
                : '[role="grid"]'
          )
          ?.focus()
      })
      return () => cancelAnimationFrame(frame)
    }
  }, [depth])

  return (
    <div
      className="pane-stack"
      data-open={open}
      data-depth={execution ? 2 : open ? 1 : 0}
      id={id}
      ref={root}
      role="tabpanel"
      aria-label={`${label} results`}
    >
      {!execution && (
        <section className="stack-pane list-pane" aria-label={`${label} list`}>
          {list}
        </section>
      )}
      <section
        className="stack-pane detail-pane"
        aria-label={`${label} details`}
        aria-hidden={!open}
        inert={!open}
      >
        {detail}
      </section>
      {execution && (
        <section
          className="stack-pane execution-pane"
          aria-label="Execution details"
        >
          {execution}
        </section>
      )}
    </div>
  )
}
