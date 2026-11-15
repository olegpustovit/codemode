import type { KeyboardEvent } from "react"
import { useLayoutEffect, useRef, useState } from "react"
import { Button } from "@/components/ui/button"
import { ErrorNotice } from "@/components/common"
export type BulkAction = {
  id: string
  label: string
  confirm?: string
  disabled?: boolean
}
export function BulkPane({
  count,
  names,
  allMatching = false,
  actions,
  onAction,
  onClose,
  confirmAction,
}: {
  count: number
  names: readonly string[]
  allMatching?: boolean
  actions: readonly BulkAction[]
  onAction: (action: string) => Promise<void>
  onClose: () => void
  confirmAction?: string
}) {
  const [confirm, setConfirm] = useState<
    (BulkAction & { run: (action: string) => Promise<void> }) | null
  >(() => {
    const action = actions.find((action) => action.id === confirmAction)
    return action ? { ...action, run: onAction } : null
  })
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<unknown>(null)
  const safe = useRef<HTMLButtonElement>(null)
  useLayoutEffect(() => {
    if (confirm) safe.current?.focus()
  }, [confirm])
  const apply = async (action: BulkAction, run = onAction) => {
    setBusy(true)
    setError(null)
    try {
      await run(action.id)
      setConfirm(null)
    } catch (error) {
      setError(error)
    } finally {
      setBusy(false)
    }
  }
  const cancelConfirmation = (event: KeyboardEvent) => {
    if (event.key === "Escape" && confirm && !busy) {
      event.preventDefault()
      event.stopPropagation()
      setConfirm(null)
    }
  }
  return (
    <>
      <div className="pane-heading">
        <span>Selection</span>
        <Button variant="ghost" onClick={onClose} disabled={busy}>
          Close <span className="kbd">Esc</span>
        </Button>
      </div>
      <section aria-label="Bulk actions" className="pane-body">
        <h2>{count} selected</h2>
        <p className="pane-meta">
          {allMatching
            ? "All matching items, including those beyond this view."
            : "Changes apply to this selection."}
        </p>
        <ul className="bulk-preview">
          {names.slice(0, 8).map((name, index) => (
            <li key={index}>{name}</li>
          ))}
        </ul>
        {count > 8 && <p className="pane-meta">And {count - 8} more.</p>}
        <ErrorNotice error={error} />
        {confirm ? (
          <>
            <p>{confirm.confirm}</p>
            <div className="detail-actions">
              <Button
                ref={safe}
                onKeyDown={cancelConfirmation}
                variant="outline"
                disabled={busy}
                onClick={() => setConfirm(null)}
              >
                Keep selection <span className="kbd">Esc</span>
              </Button>
              <Button
                variant="destructive"
                onKeyDown={cancelConfirmation}
                disabled={busy}
                onClick={() => void apply(confirm, confirm.run)}
              >
                {busy ? "Applying…" : confirm.label}
              </Button>
            </div>
          </>
        ) : (
          <div className="detail-actions">
            {actions.map((action) => (
              <Button
                key={action.id}
                variant={action.confirm ? "destructive" : "outline"}
                disabled={busy || action.disabled}
                onClick={() =>
                  action.confirm
                    ? setConfirm({ ...action, run: onAction })
                    : void apply(action)
                }
              >
                {busy ? "Applying…" : action.label}
              </Button>
            ))}
          </div>
        )}
      </section>
    </>
  )
}
