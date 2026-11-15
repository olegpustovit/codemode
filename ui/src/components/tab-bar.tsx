import { useId, useLayoutEffect, useRef } from "react"
export function TabBar({
  tabs,
  selected,
  onSelect,
  panelId,
  label = "Filter integrations",
}: {
  tabs: readonly { id: string; label: string; count: number }[]
  selected: string
  onSelect: (id: string) => void
  label?: string
  panelId: string
}) {
  const prefix = useId()
  const root = useRef<HTMLDivElement>(null)
  const line = useRef<HTMLSpanElement>(null)
  useLayoutEffect(() => {
    const place = () => {
      const tab = root.current?.querySelector<HTMLElement>(
        '[aria-selected="true"]'
      )
      if (!tab || !line.current) return
      line.current.style.transform = `translateX(${tab.offsetLeft}px)`
      line.current.style.width = `${tab.offsetWidth}px`
    }
    place()
    window.addEventListener("resize", place)
    return () => window.removeEventListener("resize", place)
  }, [selected, tabs])
  return (
    <div className="tab-bar" role="tablist" aria-label={label} ref={root}>
      {tabs.map((tab, index) => (
        <button
          key={tab.id}
          role="tab"
          aria-label={`${tab.label} ${tab.count}`}
          id={`${prefix}-${tab.id}`}
          aria-controls={panelId}
          aria-selected={tab.id === selected}
          tabIndex={tab.id === selected ? 0 : -1}
          onClick={() => onSelect(tab.id)}
          onKeyDown={(event) => {
            let next = index
            if (event.key === "ArrowRight") next = (index + 1) % tabs.length
            else if (event.key === "ArrowLeft")
              next = (index + tabs.length - 1) % tabs.length
            else if (event.key === "Home") next = 0
            else if (event.key === "End") next = tabs.length - 1
            else return
            event.preventDefault()
            onSelect(tabs[next].id)
            document.getElementById(`${prefix}-${tabs[next].id}`)?.focus()
          }}
        >
          {tab.label}
          <sup>{tab.count}</sup>
        </button>
      ))}
      <span className="tab-underline" ref={line} aria-hidden="true" />
    </div>
  )
}
