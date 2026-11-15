/* oxlint-disable jsx-a11y/no-noninteractive-element-to-interactive-role -- The WAI-ARIA grid pattern uses native table cells and one active-descendant focus. */
import { Fragment, useId, useLayoutEffect, useRef } from "react"
import type { ReactNode, RefObject } from "react"
import { useListNavigation } from "@/hooks/use-list-navigation"
export type ListRow = {
  id: string
  label: string
  group: string
  cells: ReactNode[]
}
export function DataList({
  rows,
  columns,
  className = "",
  selectedId,
  onSelect,
  onOpen,
  onBack,
  label,
  busy = false,
  listRef,
  empty = "No items match this view.",
}: {
  rows: readonly ListRow[]
  className?: string
  columns: readonly ReactNode[]
  selectedId?: string
  onSelect: (id: string) => void
  onOpen: (id: string) => void
  onBack: () => void
  label: string
  busy?: boolean
  listRef?: RefObject<HTMLTableElement | null>
  empty?: string
}) {
  const prefix = useId()
  const ownRef = useRef<HTMLTableElement>(null)
  const root = listRef ?? ownRef
  const cursor = useRef<HTMLTableRowElement>(null)
  const selected = rows.findIndex((row) => row.id === selectedId)
  const active = selected < 0 ? 0 : selected
  const { onKeyDown } = useListNavigation(
    rows.map((row) => row.id),
    rows[active]?.id,
    onSelect,
    onOpen,
    onBack
  )
  useLayoutEffect(() => {
    const place = () => {
      const row = document.getElementById(`${prefix}-${active}`)
      if (!row || !cursor.current) return
      cursor.current.style.transform = `translateY(${row.offsetTop}px)`
      cursor.current.style.height = `${row.offsetHeight}px`
      if (document.activeElement === root.current)
        row.scrollIntoView?.({ block: "nearest" })
    }
    place()
    window.addEventListener("resize", place)
    return () => window.removeEventListener("resize", place)
  }, [rows, active, prefix, root])
  return (
    <div className={`data-list ${className}`}>
      <table
        className="data-grid"
        role="grid"
        aria-label={label}
        aria-busy={busy}
        aria-colcount={columns.length}
        aria-activedescendant={rows.length ? `${prefix}-${active}` : undefined}
        tabIndex={0}
        ref={root}
        onKeyDown={onKeyDown}
      >
        <thead>
          <tr className="data-head">
            {columns.map((column, i) => (
              <th scope="col" className={i ? "data-extra" : undefined} key={i}>
                {column}
              </th>
            ))}
          </tr>
        </thead>
        <tbody className="data-body">
          {rows.length ? (
            <tr className="data-cursor" ref={cursor} aria-hidden="true">
              <td aria-hidden="true" colSpan={columns.length} />
            </tr>
          ) : null}
          {rows.map((row, index) => {
            const first = index === 0 || rows[index - 1].group !== row.group
            return (
              <Fragment key={row.id}>
                {first && (
                  <tr className="data-group">
                    <td colSpan={columns.length}>{row.group}</td>
                  </tr>
                )}
                <tr
                  id={`${prefix}-${index}`}
                  className="data-row"
                  aria-label={row.label}
                  aria-selected={index === active}
                  onKeyDown={onKeyDown}
                  onClick={() => {
                    root.current?.focus()
                    onOpen(row.id)
                  }}
                >
                  {row.cells.map((cell, i) => (
                    <td className={i ? "data-extra" : "data-name"} key={i}>
                      {cell}
                    </td>
                  ))}
                </tr>
              </Fragment>
            )
          })}
        </tbody>
      </table>
      {!rows.length && <p className="list-empty">{empty}</p>}
    </div>
  )
}
