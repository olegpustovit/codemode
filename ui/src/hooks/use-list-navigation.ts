import { useState } from "react"
import type { KeyboardEvent } from "react"

export type ListSelection = {
  ids: readonly string[]
  anchor?: string
  allMatching: boolean
}
const emptySelection: ListSelection = { ids: [], allMatching: false }
export function selectRows(
  selection: ListSelection,
  ids: readonly string[],
  cursor: string | undefined,
  target: string,
  mode: "single" | "range" | "toggle"
): ListSelection {
  if (mode === "single")
    return { ids: [target], anchor: target, allMatching: false }
  if (mode === "range") {
    const anchor = selection.anchor ?? cursor ?? target
    const from = Math.max(0, ids.indexOf(anchor))
    const to = ids.indexOf(target)
    return {
      ids: ids.slice(Math.min(from, to), Math.max(from, to) + 1),
      anchor,
      allMatching: false,
    }
  }
  const selected = new Set(selection.ids)
  if (selected.has(target)) selected.delete(target)
  else selected.add(target)
  return { ids: [...selected], anchor: target, allMatching: false }
}
export function selectAllRows(
  selection: ListSelection,
  ids: readonly string[]
): ListSelection {
  return {
    ids: [...ids],
    anchor: ids[0],
    allMatching:
      ids.length > 0 && ids.every((id) => selection.ids.includes(id)),
  }
}
export function useListSelection(scope: string) {
  const [state, setState] = useState({ scope, value: emptySelection })
  if (state.scope !== scope) setState({ scope, value: emptySelection })
  return {
    value: state.scope === scope ? state.value : emptySelection,
    set: (value: ListSelection) => setState({ scope, value }),
  }
}
export function useListCursor(initial: string | undefined, scope = "") {
  const key = `${scope}:${initial ?? ""}`
  const [state, setState] = useState({ key, id: initial })
  if (state.key !== key) setState({ key, id: initial })
  return [
    state.key === key ? state.id : initial,
    (id: string) => setState({ key, id }),
  ] as const
}
export function useListNavigation(
  ids: readonly string[],
  selectedId: string | undefined,
  onSelect: (id: string) => void,
  onOpen: (id: string) => void,
  onBack: () => void,
  options: {
    selection?: ReturnType<typeof useListSelection>
    onRename?: () => void
    onDelete?: () => void
    onAction?: (key: string) => boolean
    onLoadMore?: () => void
  } = {}
) {
  const index = Math.max(0, ids.indexOf(selectedId ?? ""))
  const choose = (
    id: string,
    mode: "single" | "range" | "toggle" = "single"
  ) => {
    options.selection?.set(
      selectRows(options.selection.value, ids, selectedId, id, mode)
    )
    onSelect(id)
  }
  const move = (amount: number, extend = false) => {
    const id = ids[Math.max(0, Math.min(ids.length - 1, index + amount))]
    if (id) choose(id, extend ? "range" : "single")
    if (amount > 0 && index === ids.length - 1) options.onLoadMore?.()
  }
  const onKeyDown = (event: KeyboardEvent) => {
    if (
      event.defaultPrevented ||
      event.altKey ||
      (event.target instanceof Element &&
        event.target.closest(
          "input,textarea,select,button,a,[contenteditable=true]"
        ))
    )
      return
    if (event.ctrlKey || event.metaKey) {
      if (event.key.toLowerCase() !== "a" || !options.selection || !ids.length)
        return
      options.selection.set(selectAllRows(options.selection.value, ids))
    } else if (event.key === "ArrowDown") move(1, event.shiftKey)
    else if (event.key === "ArrowUp") move(-1, event.shiftKey)
    else if (event.key === "Home" && ids[0])
      choose(ids[0], event.shiftKey ? "range" : "single")
    else if (event.key === "End" && ids.length) {
      choose(ids[ids.length - 1], event.shiftKey ? "range" : "single")
      options.onLoadMore?.()
    } else if (event.key === " " && ids[index] && options.selection)
      choose(ids[index], "toggle")
    else if (
      (event.key === "Enter" || event.key === "ArrowRight") &&
      ids[index]
    )
      onOpen(ids[index])
    else if (event.key === "Escape" || event.key === "ArrowLeft") {
      if (
        options.selection &&
        (options.selection.value.allMatching ||
          options.selection.value.ids.length > 1) &&
        ids[index]
      )
        choose(ids[index])
      else onBack()
    } else if (event.key === "F2" && ids[index] && options.onRename)
      options.onRename()
    else if (
      (event.key === "Delete" || event.key === "Backspace") &&
      ids[index] &&
      options.onDelete
    )
      options.onDelete()
    else if (!options.onAction?.(event.key.toLowerCase())) return
    event.preventDefault()
  }
  return { onKeyDown, move, choose }
}

export function listSearchKeys(
  event: KeyboardEvent,
  list: HTMLElement | null,
  clear: () => void
) {
  if (!["ArrowDown", "Enter", "Escape"].includes(event.key)) return
  event.preventDefault()
  if (event.key === "Escape") clear()
  list?.focus()
}
