import type { KeyboardEvent } from "react"
export function useListNavigation(
  ids: readonly string[],
  selectedId: string | undefined,
  onSelect: (id: string) => void,
  onOpen: (id: string) => void,
  onBack: () => void
) {
  const index = Math.max(0, ids.indexOf(selectedId ?? ""))
  const move = (amount: number) => {
    const id = ids[Math.max(0, Math.min(ids.length - 1, index + amount))]
    if (id && id !== selectedId) onSelect(id)
  }
  const onKeyDown = (event: KeyboardEvent) => {
    if (
      event.defaultPrevented ||
      event.altKey ||
      event.ctrlKey ||
      event.metaKey
    )
      return
    if (event.key === "ArrowDown") move(1)
    else if (event.key === "ArrowUp") move(-1)
    else if (event.key === "Home" && ids[0]) onSelect(ids[0])
    else if (event.key === "End" && ids.length) onSelect(ids[ids.length - 1])
    else if (
      (event.key === "Enter" || event.key === "ArrowRight") &&
      ids[index]
    )
      onOpen(ids[index])
    else if (event.key === "Escape" || event.key === "ArrowLeft") onBack()
    else return
    event.preventDefault()
  }
  return { onKeyDown, move }
}
