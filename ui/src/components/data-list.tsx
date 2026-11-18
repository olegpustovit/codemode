/* oxlint-disable jsx-a11y/no-noninteractive-element-to-interactive-role -- The WAI-ARIA grid pattern uses native table cells and one active-descendant focus. */
import { Fragment, useId, useLayoutEffect, useRef, useState } from "react"
import type { ReactNode, RefObject } from "react"
import {
  useListNavigation,
  useListSelection,
} from "@/hooks/use-list-navigation"
import { message } from "@/lib/api"
import {
  Empty,
  EmptyHeader,
  EmptyDescription,
  EmptyTitle,
  EmptyMedia,
  EmptyContent,
} from "@/components/ui/empty"
export function ListEmpty({
  title,
  description,
  icon,
  children,
}: {
  title: string
  description: string
  icon?: ReactNode
  children?: ReactNode
}) {
  return (
    <>
      <EmptyHeader>
        {icon && <EmptyMedia>{icon}</EmptyMedia>}
        <EmptyTitle>{title}</EmptyTitle>
        <EmptyDescription>{description}</EmptyDescription>
      </EmptyHeader>
      {children && (
        <EmptyContent className="empty-actions">{children}</EmptyContent>
      )}
    </>
  )
}
export type ListRow = {
  id: string
  label: string
  group: string
  cells: ReactNode[]
  renameable?: boolean
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
  selection: sharedSelection,
  selectionScope = label,
  totalMatches = rows.length,
  onRename,
  onDelete,
  onAction,
  onLoadMore,
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
  empty?: ReactNode
  selection?: ReturnType<typeof useListSelection>
  selectionScope?: string
  totalMatches?: number
  onRename?: (id: string, name: string) => Promise<void>
  onDelete?: () => void
  onAction?: (key: string) => boolean
  onLoadMore?: () => void
}) {
  const prefix = useId()
  const ownRef = useRef<HTMLTableElement>(null)
  const root = listRef ?? ownRef
  const cursor = useRef<HTMLTableRowElement>(null)
  const selected = rows.findIndex((row) => row.id === selectedId)
  const active = selected < 0 ? 0 : selected
  const ownSelection = useListSelection(selectionScope)
  const selection = sharedSelection ?? ownSelection
  const [touchSelection, setTouchSelection] = useState(false)
  const [rename, setRename] = useState<{ id: string; value: string } | null>(
    null
  )
  const [saving, setSaving] = useState(false)
  const [renameError, setRenameError] = useState("")
  const requestedLength = useRef<number | null>(null)
  const startRename = () => {
    const row = rows[active]
    if (row && onRename && row.renameable !== false) {
      setRename({ id: row.id, value: row.label })
      setRenameError("")
    }
  }
  function openRow(id: string) {
    onOpen(id)
    if (
      typeof window.requestAnimationFrame !== "function" ||
      window.matchMedia?.("(prefers-reduced-motion: reduce)")?.matches
    )
      return
    const index = rows.findIndex((row) => row.id === id)
    const row = document.getElementById(`${prefix}-${index}`)
    requestAnimationFrame(() => {
      if (!row?.isConnected) return
      row.animate?.(
        [
          { backgroundPosition: "100% 0", backgroundImage: "var(--row-sweep)" },
          { backgroundPosition: "0 0", backgroundImage: "var(--row-sweep)" },
        ],
        { duration: 700, easing: "cubic-bezier(.22, 1, .36, 1)" }
      )
    })
  }
  const { onKeyDown, choose } = useListNavigation(
    rows.map((row) => row.id),
    rows[active]?.id,
    onSelect,
    openRow,
    onBack,
    {
      selection,
      onRename: onRename ? startRename : undefined,
      onDelete,
      onAction,
      onLoadMore: onLoadMore
        ? () => {
            requestedLength.current = rows.length
            onLoadMore()
          }
        : undefined,
    }
  )
  useLayoutEffect(() => {
    const previous = requestedLength.current
    if (previous !== null && rows.length > previous) {
      requestedLength.current = null
      choose(rows[previous].id)
    }
  }, [rows, choose])
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
      {!!rows.length && (
        <div className="list-selection-tools">
          <button
            className="list-select-toggle"
            aria-pressed={touchSelection}
            onClick={() => setTouchSelection(!touchSelection)}
          >
            {touchSelection ? "Done selecting" : "Select"}
          </button>
          {(selection.value.ids.length > 1 || selection.value.allMatching) && (
            <>
              <span>
                {selection.value.allMatching
                  ? totalMatches
                  : selection.value.ids.length}{" "}
                selected
              </span>
              {!selection.value.allMatching &&
                totalMatches > selection.value.ids.length && (
                  <button
                    onClick={() =>
                      selection.set({ ...selection.value, allMatching: true })
                    }
                  >
                    Select all {totalMatches} matching
                  </button>
                )}
            </>
          )}
        </div>
      )}
      {renameError && (
        <p role="alert" className="bad">
          {renameError}
        </p>
      )}
      <table
        className="data-grid"
        role="grid"
        aria-label={label}
        aria-busy={busy}
        aria-multiselectable="true"
        aria-colcount={columns.length}
        aria-activedescendant={rows.length ? `${prefix}-${active}` : undefined}
        tabIndex={0}
        ref={root}
        onKeyDown={onKeyDown}
        onScrollCapture={(event) => {
          const body = event.target
          if (
            body instanceof HTMLElement &&
            body.classList.contains("data-body") &&
            body.scrollHeight - body.scrollTop - body.clientHeight < 120
          )
            onLoadMore?.()
        }}
      >
        {rows.length > 0 && (
          <thead>
            <tr className="data-head">
              {columns.map((column, i) => (
                <th
                  scope="col"
                  className={i ? "data-extra" : undefined}
                  key={i}
                >
                  {column}
                </th>
              ))}
            </tr>
          </thead>
        )}
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
                  aria-selected={
                    selection.value.allMatching ||
                    (selection.value.ids.length
                      ? selection.value.ids.includes(row.id)
                      : index === active)
                  }
                  data-active={index === active}
                  onKeyDown={onKeyDown}
                  onClick={(event) => {
                    if (
                      event.target instanceof Element &&
                      event.target.closest("button,a,input,select,textarea")
                    )
                      return
                    root.current?.focus()
                    if (event.shiftKey) choose(row.id, "range")
                    else if (event.metaKey || event.ctrlKey || touchSelection)
                      choose(row.id, "toggle")
                    else {
                      choose(row.id)
                      openRow(row.id)
                    }
                  }}
                >
                  {row.cells.map((cell, i) => (
                    <td className={i ? "data-extra" : "data-name"} key={i}>
                      {i === 0 && touchSelection && (
                        <input
                          type="checkbox"
                          aria-label={`Select ${row.label}`}
                          checked={
                            selection.value.allMatching ||
                            selection.value.ids.includes(row.id)
                          }
                          onChange={() => choose(row.id, "toggle")}
                        />
                      )}
                      {i === 0 && rename?.id === row.id ? (
                        <input
                          className="row-rename"
                          aria-label={`Rename ${row.label}`}
                          value={rename.value}
                          ref={(node) => {
                            node?.focus()
                          }}
                          maxLength={120}
                          disabled={saving}
                          onChange={(event) =>
                            setRename({ ...rename, value: event.target.value })
                          }
                          onKeyDown={(event) => {
                            event.stopPropagation()
                            if (event.key === "Escape") {
                              event.preventDefault()
                              setRename(null)
                              root.current?.focus()
                            } else if (
                              event.key === "Enter" &&
                              onRename &&
                              !saving
                            ) {
                              event.preventDefault()
                              setSaving(true)
                              void onRename(rename.id, rename.value)
                                .then(() => {
                                  setRename(null)
                                  root.current?.focus()
                                })
                                .catch((error) =>
                                  setRenameError(message(error))
                                )
                                .finally(() => setSaving(false))
                            }
                          }}
                        />
                      ) : (
                        cell
                      )}
                    </td>
                  ))}
                </tr>
              </Fragment>
            )
          })}
        </tbody>
      </table>
      {!rows.length && (
        <Empty className="list-empty">
          {typeof empty === "string" ? (
            <EmptyHeader>
              <EmptyTitle>
                {busy
                  ? `Loading ${label.toLowerCase()}…`
                  : `No ${label.toLowerCase()}`}
              </EmptyTitle>
              <EmptyDescription>{empty}</EmptyDescription>
            </EmptyHeader>
          ) : (
            empty
          )}
        </Empty>
      )}
    </div>
  )
}
